//! Public exchange rate endpoint.
//!
//! Exposes the configured rate provider so frontends can show
//! "you will pay X ETH for $Y USD" before creating an invoice.

use axum::{Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use auth::SessionService;
use rates::RateError;

use super::extractors::AuthenticatedUser;
use crate::state::PgAppState;

/// Query parameters for the rates endpoint.
#[derive(Debug, Deserialize, IntoParams)]
pub struct GetRateQuery {
    /// Source currency (e.g., "USD", "EUR").
    pub from: String,
    /// Target currency (e.g., "ETH", "USDC").
    pub to: String,
}

/// Exchange rate response.
#[derive(Debug, Serialize, ToSchema)]
pub struct RateResponse {
    /// Source currency.
    pub from: String,
    /// Target currency.
    pub to: String,
    /// Exchange rate: 1 unit of `from` = `rate` units of `to`.
    pub rate: String,
    /// When the rate was fetched (ISO 8601).
    pub timestamp: String,
    /// Age of the rate in seconds.
    pub age_seconds: i64,
}

/// Get the current exchange rate for a currency pair.
///
/// Returns the live rate from the configured provider (Kraken with
/// CoinGecko fallback). Rates are cached for ~30 s by the provider.
#[utoipa::path(
    get,
    path = "/rates",
    tag = "rates",
    security(("bearer_auth" = [])),
    params(GetRateQuery),
    responses(
        (status = 200, description = "Exchange rate", body = RateResponse),
        (status = 400, description = "Unsupported currency pair"),
        (status = 401, description = "Unauthorized"),
        (status = 503, description = "Rate provider unavailable"),
    )
)]
pub async fn get_rate<A>(
    AuthenticatedUser(_user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    axum::extract::Query(query): axum::extract::Query<GetRateQuery>,
) -> Result<Json<RateResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    let exchange_rate = state
        .rate_provider
        .get_rate(&query.from, &query.to)
        .await
        .map_err(|e| match &e {
            RateError::UnsupportedPair { .. } => {
                tracing::debug!(from = %query.from, to = %query.to, "unsupported pair");
                StatusCode::BAD_REQUEST
            }
            _ => {
                tracing::warn!(from = %query.from, to = %query.to, error = %e, "rate fetch failed");
                StatusCode::SERVICE_UNAVAILABLE
            }
        })?;

    let age_seconds = (chrono::Utc::now() - exchange_rate.timestamp)
        .num_seconds()
        .max(0);

    Ok(Json(RateResponse {
        from: exchange_rate.from,
        to: exchange_rate.to,
        rate: exchange_rate.rate.to_string(),
        timestamp: exchange_rate.timestamp.to_rfc3339(),
        age_seconds,
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn rate_response_serialization() {
        let resp = RateResponse {
            from: "USD".to_string(),
            to: "ETH".to_string(),
            rate: "0.000421".to_string(),
            timestamp: "2026-05-03T12:00:00+00:00".to_string(),
            age_seconds: 5,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["from"], "USD");
        assert_eq!(json["to"], "ETH");
        assert_eq!(json["rate"], "0.000421");
        assert_eq!(json["age_seconds"], 5);
    }
}
