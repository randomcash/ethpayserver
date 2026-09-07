//! Dashboard payment analytics (RCS-225).
//!
//! Serves both the volume chart and the payment-methods breakdown from one
//! aggregation, because they are the same grouping read two ways.
//!
//! # Units
//!
//! Volume is reported **per asset, in that asset's own units**. There is no
//! combined total, and that is deliberate: 1 ETH + 1 USDC is not 2 of
//! anything, so an unlabelled mixed number would be worse than no number.
//!
//! Converting through the `rates` crate was the alternative and it does not
//! hold up for a history chart. `rates` is a *live* provider, so applying
//! today's price to a payment from three weeks ago restates history every time
//! the page is refreshed. `payments.credited_amount` is the historically
//! correct conversion, but it is denominated in the *invoice's* currency,
//! which differs per invoice, and it is NULL whenever conversion failed — so
//! summing it would mix currencies and silently drop payments.
//!
//! The one figure that *is* summable across assets is a payment count, so the
//! methods breakdown is a share of payments received, not a share of value.

use std::collections::BTreeMap;
use std::str::FromStr;

use axum::{Json, extract::State, http::StatusCode};
use chrono::{DateTime, Duration, NaiveDate, NaiveTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use auth::{SessionService, repository::StoreRepository};
use data_service::{PaymentAnalyticsReader, PaymentVolumeBucket, PaymentVolumeQuery};

use crate::api::extractors::AuthenticatedUser;
use crate::state::PgAppState;

/// Window used when the caller does not ask for one.
const DEFAULT_WINDOW_DAYS: u32 = 30;

/// Longest window the endpoint will aggregate.
///
/// The chart offers 7D/30D/90D, and the response carries one point per day per
/// asset, so an unbounded `days` would be both a scan over all history and an
/// unbounded response body.
const MAX_WINDOW_DAYS: u32 = 90;

/// Query parameters for the analytics endpoint.
#[derive(Debug, Deserialize, IntoParams)]
pub struct AnalyticsQuery {
    /// Size of the window in days, ending today (UTC). 1..=90, default 30.
    pub days: Option<u32>,
}

/// One day of volume for a single asset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct DailyVolume {
    /// UTC calendar day.
    pub date: NaiveDate,
    /// Volume for the day in whole units of the asset (e.g. `"0.75"` ETH).
    pub amount: String,
    /// Payments received that day.
    pub payment_count: i64,
}

/// Volume for one asset over the whole window.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct AssetVolume {
    /// Asset symbol as recorded on the payments (e.g. `ETH`).
    pub asset_symbol: String,
    /// Window total in whole units of this asset. Only comparable to other
    /// values of the same `asset_symbol`.
    pub total_amount: String,
    /// Payments received in this asset over the window.
    pub payment_count: i64,
    /// This asset's share of the window's payment *count*, 0.0..=100.0.
    pub share_percent: f64,
    /// One entry per day of the window, ascending, zero-filled.
    pub daily: Vec<DailyVolume>,
}

/// Payment analytics for the authenticated user's stores.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct DashboardAnalytics {
    /// Window size actually used.
    pub days: u32,
    /// First day of the window (UTC, inclusive).
    pub start_date: NaiveDate,
    /// Last day of the window (UTC, inclusive).
    pub end_date: NaiveDate,
    /// Payments received across all assets in the window.
    pub total_payments: i64,
    /// Per-asset series, busiest asset first. Empty when nothing was received
    /// — an account with no payments gets `[]`, never a fabricated series.
    pub assets: Vec<AssetVolume>,
}

/// Get per-day, per-asset payment volume for the authenticated user's stores.
#[utoipa::path(
    get,
    path = "/dashboard/analytics",
    tag = "dashboard",
    security(("bearer_auth" = [])),
    params(AnalyticsQuery),
    responses(
        (status = 200, description = "Payment analytics", body = DashboardAnalytics),
        (status = 400, description = "days out of range (1..=90)"),
        (status = 401, description = "Unauthorized"),
    )
)]
pub async fn get_analytics<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    axum::extract::Query(query): axum::extract::Query<AnalyticsQuery>,
) -> Result<Json<DashboardAnalytics>, StatusCode>
where
    A: SessionService + 'static,
{
    let days = validate_days(query.days)?;
    let ds = &*state.data_service;

    // Scoped exactly like `/dashboard/stats`: the stores this user is a member
    // of, and nothing else. An empty list is a real answer (a brand-new
    // account), not a licence to read every store — see `PaymentVolumeQuery`.
    let stores: Vec<auth::Store> = ds
        .get_stores_for_user(user.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let store_ids = stores.iter().map(|s| auth::StoreId(s.id.0)).collect();

    let end_date = Utc::now().date_naive();
    let start_date = end_date - Duration::days(i64::from(days) - 1);

    let buckets = ds
        .payment_volume_by_day(&PaymentVolumeQuery {
            store_ids,
            since: start_of_utc_day(start_date),
            until: start_of_utc_day(end_date + Duration::days(1)),
        })
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "payment volume aggregation failed");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(build_analytics(&buckets, start_date, days)))
}

/// Reject a window we are not willing to aggregate, rather than silently
/// serving a different one than the caller asked for.
fn validate_days(days: Option<u32>) -> Result<u32, StatusCode> {
    match days.unwrap_or(DEFAULT_WINDOW_DAYS) {
        d if (1..=MAX_WINDOW_DAYS).contains(&d) => Ok(d),
        _ => Err(StatusCode::BAD_REQUEST),
    }
}

fn start_of_utc_day(date: NaiveDate) -> DateTime<Utc> {
    date.and_time(NaiveTime::MIN).and_utc()
}

/// Running totals for one asset while folding the buckets.
#[derive(Default)]
struct AssetAccumulator {
    total: Decimal,
    payment_count: i64,
    per_day: BTreeMap<NaiveDate, (Decimal, i64)>,
}

/// Fold repository buckets into the response, zero-filling every day of the
/// window so the chart never has to guess where a gap is.
fn build_analytics(
    buckets: &[PaymentVolumeBucket],
    start_date: NaiveDate,
    days: u32,
) -> DashboardAnalytics {
    let mut assets: BTreeMap<String, AssetAccumulator> = BTreeMap::new();
    let mut total_payments: i64 = 0;

    for bucket in buckets {
        let Some(amount) = scale_amount(bucket) else {
            continue;
        };
        let acc = assets.entry(bucket.asset_symbol.clone()).or_default();
        acc.total = acc.total.saturating_add(amount);
        acc.payment_count += bucket.payment_count;
        let day = acc.per_day.entry(bucket.day).or_default();
        day.0 = day.0.saturating_add(amount);
        day.1 += bucket.payment_count;
        total_payments += bucket.payment_count;
    }

    let end_date = start_date + Duration::days(i64::from(days) - 1);
    let mut assets: Vec<AssetVolume> = assets
        .into_iter()
        .map(|(symbol, acc)| finish_asset(symbol, acc, start_date, days, total_payments))
        .collect();

    // Busiest asset first so the client can default the chart to the one the
    // merchant actually uses; symbol breaks ties so the order is stable.
    assets.sort_by(|a, b| {
        b.payment_count
            .cmp(&a.payment_count)
            .then_with(|| a.asset_symbol.cmp(&b.asset_symbol))
    });

    DashboardAnalytics {
        days,
        start_date,
        end_date,
        total_payments,
        assets,
    }
}

fn finish_asset(
    asset_symbol: String,
    acc: AssetAccumulator,
    start_date: NaiveDate,
    days: u32,
    total_payments: i64,
) -> AssetVolume {
    let daily = (0..i64::from(days))
        .map(|offset| {
            let date = start_date + Duration::days(offset);
            let (amount, payment_count) = acc.per_day.get(&date).cloned().unwrap_or_default();
            DailyVolume {
                date,
                amount: format_amount(amount),
                payment_count,
            }
        })
        .collect();

    let share_percent = if total_payments > 0 {
        // One decimal place: shares are read off a bar, and more precision
        // than that is noise.
        let raw = acc.payment_count as f64 / total_payments as f64 * 100.0;
        (raw * 10.0).round() / 10.0
    } else {
        0.0
    };

    AssetVolume {
        asset_symbol,
        total_amount: format_amount(acc.total),
        payment_count: acc.payment_count,
        share_percent,
        daily,
    }
}

/// Convert a smallest-unit sum into whole units of the asset.
///
/// `set_scale` is exact where a division would round: the mantissa is
/// untouched and only the decimal point moves. Returns `None` for a sum too
/// large for a 96-bit mantissa or a decimals value past `Decimal`'s limit —
/// neither is a payment anyone actually received, and dropping that one bucket
/// beats failing the whole panel.
fn scale_amount(bucket: &PaymentVolumeBucket) -> Option<Decimal> {
    let mut amount = Decimal::from_str(&bucket.raw_amount)
        .inspect_err(|e| {
            tracing::warn!(
                asset = %bucket.asset_symbol,
                raw_amount = %bucket.raw_amount,
                error = %e,
                "skipping unrepresentable payment volume bucket",
            );
        })
        .ok()?;
    amount
        .set_scale(u32::from(bucket.decimals))
        .inspect_err(|e| {
            tracing::warn!(
                asset = %bucket.asset_symbol,
                decimals = bucket.decimals,
                error = %e,
                "skipping payment volume bucket with unusable decimals",
            );
        })
        .ok()?;
    Some(amount)
}

/// Render an amount without the trailing zeros the scaling introduces, so the
/// client shows `0.5` rather than `0.500000000000000000`.
fn format_amount(amount: Decimal) -> String {
    amount.normalize().to_string()
}

#[cfg(test)]
mod tests;
