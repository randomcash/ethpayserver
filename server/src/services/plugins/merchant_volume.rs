//! Capability 6: how much one account settled over a window, as one number.
//!
//! Our pricing brackets are denominated in settled volume, and a plugin has
//! no way to compute that for itself. It sees its own schema and the payments
//! on the instance's own store; a merchant's own takings are neither. Before
//! this, the billing plugin summed the subscription invoices it had issued
//! and called the result "settled volume" - so a merchant on a plan with a
//! six-figure cap read as having used the price of their own subscription,
//! and no bracket could ever be reached.
//!
//! # What this deliberately is not
//!
//! The alternative shape is the one `payment_observer` refuses: show the
//! plugin every payment every merchant received and let it add them up. That
//! discloses amounts, assets, chains and customer-supplied metadata for every
//! merchant on the instance, to answer a question that needs one number per
//! account. So this returns the number and nothing else.
//!
//! It is still a disclosure, and worth naming as one: a plugin with this
//! capability can learn any account's settled volume. It cannot learn what
//! any individual payment was.
//!
//! # Owned stores, not every store a merchant can see
//!
//! [`StoreRepository::get_stores_owned_by`], not `get_stores_for_user`. A
//! merchant who is a *member* of someone else's store did not take that
//! money, and counting it would bill the same volume twice - once to the
//! owner and once to every collaborator - which is how a merchant gets a bill
//! for a colleague's turnover.
//!
//! # Unpriceable assets are dropped, and the direction matters
//!
//! An asset the rate provider cannot quote is left out of the sum rather than
//! failing the whole read. That makes the answer an *undercount*, never an
//! overcount, and an undercount can only ever under-bill or refuse less - it
//! can never bill a merchant for volume they did not do, or refuse one who is
//! inside their bracket. The symbols that were dropped come back with the
//! answer so an operator can see a rate feed is missing rather than wondering
//! why a merchant's gauge reads low.

use std::collections::BTreeSet;

use async_trait::async_trait;
use auth::SessionService;
use auth::repository::StoreRepository;
use chrono::{Duration, Utc};
use data_service::{PaymentAnalyticsReader, PaymentVolumeQuery};
use rust_decimal::Decimal;
use types::{StoreId, UserId};

use crate::state::PgAppState;

/// The longest window this capability will aggregate over.
///
/// A plugin names the window, so it is the plugin that would otherwise be
/// able to ask for all history on every call. A rolling billing window is
/// days to weeks; a year is already far past anything a bracket is computed
/// on, and is the clamp rather than a refusal so a plugin asking for too much
/// gets an answer it can use instead of an error it has to handle.
pub const MAX_WINDOW_DAYS: u32 = 366;

/// One account's settled volume, quoted in a single currency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerchantVolume {
    /// The sum, as a decimal string. A string rather than a number because
    /// every value crossing the plugin boundary is text: a JSON number cannot
    /// carry the precision these columns hold.
    pub volume: String,
    /// The currency `volume` is quoted in, echoed back so a plugin never has
    /// to assume the host honoured the currency it asked for.
    pub currency: String,
    /// Asset symbols that were present in the window and could not be priced,
    /// and are therefore missing from `volume`. Empty is the ordinary case.
    pub unpriced_assets: Vec<String>,
}

/// Read an account's settled volume over a window.
#[async_trait]
pub trait MerchantVolumeReader: Send + Sync {
    /// Sum what `account_id` settled in the `window_days` before now, across
    /// the stores it owns, quoted in `currency`.
    ///
    /// # Errors
    /// A message describing what could not be read.
    async fn merchant_volume(
        &self,
        account_id: UserId,
        window_days: u32,
        currency: &str,
    ) -> Result<MerchantVolume, String>;
}

/// The capability, over the instance's data service and rate provider.
///
/// Its own type rather than a method on `PluginHostApi`, because that one is
/// built around the instance's own store and cannot be constructed without
/// one. Reading a merchant's volume needs no own store - an instance that
/// sells nothing still has merchants with volume - and tying the two together
/// would make this capability disappear for a reason that has nothing to do
/// with it.
pub struct PluginMerchantVolume<A> {
    state: PgAppState<A>,
}

impl<A> PluginMerchantVolume<A> {
    #[must_use]
    pub fn new(state: PgAppState<A>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl<A: SessionService + 'static> MerchantVolumeReader for PluginMerchantVolume<A> {
    async fn merchant_volume(
        &self,
        account_id: UserId,
        window_days: u32,
        currency: &str,
    ) -> Result<MerchantVolume, String> {
        let until = Utc::now();
        let since = until - Duration::days(i64::from(window_days.clamp(1, MAX_WINDOW_DAYS)));

        let stores: Vec<StoreId> =
            StoreRepository::get_stores_owned_by(&*self.state.data_service, account_id)
                .await
                .map_err(|e| format!("could not read this account's stores: {e}"))?
                .into_iter()
                .map(|store| store.id)
                .collect();

        // An empty store list means an empty result, and `PaymentVolumeQuery`
        // says so explicitly: it has no "every store" variant, so there is
        // nothing here that could turn "this account owns nothing" into "read
        // the whole instance".
        if stores.is_empty() {
            return Ok(MerchantVolume {
                volume: "0".to_string(),
                currency: currency.to_string(),
                unpriced_assets: Vec::new(),
            });
        }

        let buckets = PaymentAnalyticsReader::payment_volume_by_day(
            &*self.state.data_service,
            &PaymentVolumeQuery {
                store_ids: stores,
                since,
                until,
            },
        )
        .await
        .map_err(|e| format!("could not read this account's payment volume: {e}"))?;

        let per_asset = collapse_by_asset(buckets);

        // Rates are resolved first and the arithmetic done after, so the part
        // that can be wrong in a way nobody notices - the direction of the
        // conversion - is a pure function a test can pin. Quoting an asset by
        // multiplying instead of dividing produces a perfectly plausible
        // number, and no integration test that seeds one asset would catch
        // it.
        let mut rates: std::collections::BTreeMap<String, Decimal> =
            std::collections::BTreeMap::new();
        for (asset, _) in per_asset.keys() {
            if rates.contains_key(asset) || asset.eq_ignore_ascii_case(currency) {
                continue;
            }
            match self.state.rate_provider.get_rate(currency, asset).await {
                Ok(rate) => {
                    rates.insert(asset.clone(), rate.rate);
                }
                Err(e) => tracing::warn!(
                    %asset,
                    %currency,
                    error = %e,
                    "no rate for an asset in this account's volume; it is omitted from the total"
                ),
            }
        }

        let (total, unpriced) = total_in(&per_asset, currency, |asset| rates.get(asset).copied());

        Ok(MerchantVolume {
            volume: total.normalize().to_string(),
            currency: currency.to_string(),
            unpriced_assets: unpriced.into_iter().collect(),
        })
    }
}

/// Collapse the per-day, per-asset buckets the database returns into one
/// figure per `(asset, decimals)`.
///
/// Grouping by day is more detail than a total needs, but it is what the
/// analytics reader already provides and already tests; collapsing here means
/// one rate lookup per asset rather than one per day per asset.
fn collapse_by_asset(
    buckets: Vec<data_service::PaymentVolumeBucket>,
) -> std::collections::BTreeMap<(String, u8), Decimal> {
    let mut per_asset = std::collections::BTreeMap::new();
    for bucket in buckets {
        let Ok(raw) = bucket.raw_amount.parse::<Decimal>() else {
            // The column is `numeric(78, 0)`, wider than this type. A sum that
            // does not fit is dropped rather than saturated: a saturated value
            // would read as an enormous volume and refuse a merchant outright,
            // which is the one direction this must never fail in.
            tracing::error!(
                asset = %bucket.asset_symbol,
                raw_amount = %bucket.raw_amount,
                "a payment volume bucket did not fit a decimal; it is omitted from the total"
            );
            continue;
        };
        *per_asset
            .entry((bucket.asset_symbol, bucket.decimals))
            .or_insert(Decimal::ZERO) += raw;
    }
    per_asset
}

/// The arithmetic, with the rates already resolved.
///
/// `rate_for` answers in the direction invoice creation asks in:
/// `currency -> asset`, meaning "1 `currency` buys `rate` of `asset`". So an
/// amount of the asset becomes an amount of the currency by *dividing*. That
/// is the whole conversion, and getting it backwards yields a number that
/// looks entirely reasonable - which is why it is here, in a pure function,
/// rather than inline next to an `await`.
///
/// Anything that cannot be turned into a figure honestly - no rate, a
/// non-positive rate, a shift or a division that does not fit - drops the
/// asset and names it in the second return value. Dropping under-counts, and
/// an undercount can only ever under-bill or refuse less. Guessing would do
/// the opposite.
fn total_in(
    per_asset: &std::collections::BTreeMap<(String, u8), Decimal>,
    currency: &str,
    rate_for: impl Fn(&str) -> Option<Decimal>,
) -> (Decimal, Vec<String>) {
    let mut total = Decimal::ZERO;
    let mut unpriced: BTreeSet<String> = BTreeSet::new();

    for ((asset, decimals), raw) in per_asset {
        let Some(amount) = scale(*raw, *decimals) else {
            unpriced.insert(asset.clone());
            continue;
        };

        if amount.is_zero() {
            continue;
        }

        // Already in the quote unit. No rate is consulted, so a missing feed
        // for the instance's own unit cannot make every merchant read zero.
        if asset.eq_ignore_ascii_case(currency) {
            total += amount;
            continue;
        }

        let Some(rate) = rate_for(asset).filter(|r| *r > Decimal::ZERO) else {
            unpriced.insert(asset.clone());
            continue;
        };

        match amount.checked_div(rate) {
            Some(quoted) => total += quoted,
            None => {
                unpriced.insert(asset.clone());
            }
        }
    }

    (total, unpriced.into_iter().collect())
}

/// Smallest units to whole units.
///
/// Done by moving the decimal point rather than dividing by a power of ten,
/// so the result is exact: `raw` arrives with a scale of zero (it was parsed
/// from an integer string), and setting its scale to `decimals` *is* the
/// division, with no rounding step to lose digits in.
///
/// `None` when the shift does not fit - this type carries at most 28 decimal
/// places - which drops the asset rather than reporting a number off by
/// orders of magnitude. A volume that reads too high refuses a merchant who
/// is inside their bracket, so this is the direction to fail in.
fn scale(raw: Decimal, decimals: u8) -> Option<Decimal> {
    let mut scaled = raw;
    let target = u32::from(decimals).checked_add(raw.scale())?;
    scaled.set_scale(target).ok()?;
    Some(scaled)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn at(y: i32, m: u32, d: u32) -> chrono::DateTime<Utc> {
        use chrono::TimeZone;
        Utc.with_ymd_and_hms(y, m, d, 0, 0, 0).unwrap()
    }

    #[test]
    fn smallest_units_become_whole_units() {
        // 1_500_000 units at 6 decimals is 1.5 USDC.
        assert_eq!(
            scale(Decimal::from(1_500_000u64), 6),
            Some(Decimal::from_str_exact("1.5").unwrap())
        );
        // 18 decimals, the ordinary ETH case.
        assert_eq!(
            scale(Decimal::from(1_000_000_000_000_000_000u64), 18),
            Some(Decimal::ONE)
        );
        // Zero decimals is the identity, not a division by zero.
        assert_eq!(scale(Decimal::from(42u64), 0), Some(Decimal::from(42u64)));
    }

    /// A shift this type cannot represent must drop the asset rather than
    /// produce a number that is wrong by orders of magnitude - a volume that
    /// reads too high refuses a merchant who is inside their bracket.
    #[test]
    fn a_shift_that_overflows_is_refused_rather_than_approximated() {
        assert_eq!(scale(Decimal::ONE, u8::MAX), None);
    }

    /// The window is named by the plugin, so it is the plugin that could ask
    /// for all history on every call.
    #[test]
    fn the_window_is_clamped_at_both_ends() {
        assert_eq!(0u32.clamp(1, MAX_WINDOW_DAYS), 1);
        assert_eq!(30u32.clamp(1, MAX_WINDOW_DAYS), 30);
        assert_eq!(u32::MAX.clamp(1, MAX_WINDOW_DAYS), MAX_WINDOW_DAYS);
    }

    fn dec(s: &str) -> Decimal {
        Decimal::from_str_exact(s).unwrap()
    }

    fn assets(pairs: &[(&str, u8, &str)]) -> std::collections::BTreeMap<(String, u8), Decimal> {
        pairs
            .iter()
            .map(|(a, d, raw)| (((*a).to_string(), *d), dec(raw)))
            .collect()
    }

    /// The conversion that would otherwise be wrong in a way nobody notices.
    ///
    /// The rate answers "1 USD buys `rate` ETH", so 2 ETH at 1 USD = 0.0004
    /// ETH is 5000 USD. Multiplying instead of dividing gives 0.0008 - a
    /// number that looks like a number, on a gauge nobody can sanity-check.
    #[test]
    fn an_asset_is_quoted_by_dividing_by_the_rate_not_multiplying() {
        let per_asset = assets(&[("ETH", 18, "2000000000000000000")]);
        let (total, unpriced) = total_in(&per_asset, "USD", |_| Some(dec("0.0004")));

        assert_eq!(total, dec("5000"), "2 ETH at $2500/ETH is $5000");
        assert!(unpriced.is_empty());
    }

    /// Several assets, one total. The instance's own unit is added without
    /// consulting a rate at all, so a missing USD feed cannot zero everyone.
    #[test]
    fn assets_are_summed_into_one_figure_and_the_quote_unit_needs_no_rate() {
        let per_asset = assets(&[
            ("USDC", 6, "1500000"),
            ("USD", 2, "250"),
            ("ETH", 18, "1000000000000000000"),
        ]);
        let (total, unpriced) = total_in(&per_asset, "USD", |asset| match asset {
            "USDC" => Some(dec("1")),
            "ETH" => Some(dec("0.0004")),
            _ => None,
        });

        // 1.5 USDC at parity + 2.50 USD + 1 ETH at $2500.
        assert_eq!(total, dec("2504"));
        assert!(unpriced.is_empty(), "{unpriced:?}");
    }

    /// An asset with no rate is dropped and named. Dropping under-counts,
    /// and an undercount can only under-bill or refuse less - the safe
    /// direction. Failing the whole read instead would make one missing feed
    /// take every merchant's gauge down with it.
    #[test]
    fn an_unpriceable_asset_is_dropped_named_and_never_guessed_at() {
        let per_asset = assets(&[("USDC", 6, "1000000"), ("WEIRD", 18, "5000000000000000000")]);
        let (total, unpriced) = total_in(&per_asset, "USD", |asset| {
            (asset == "USDC").then(|| dec("1"))
        });

        assert_eq!(total, dec("1"), "the priced asset still counts");
        assert_eq!(unpriced, vec!["WEIRD".to_string()]);
    }

    /// A rate provider that answers zero or a negative number is a broken
    /// feed, not a free merchant. Dividing by it would be a panic or an
    /// infinity; treating it as absent drops the asset.
    #[test]
    fn a_non_positive_rate_drops_the_asset_rather_than_dividing_by_it() {
        let per_asset = assets(&[("ETH", 18, "1000000000000000000")]);
        for rate in ["0", "-0.0004"] {
            let (total, unpriced) = total_in(&per_asset, "USD", |_| Some(dec(rate)));
            assert_eq!(total, Decimal::ZERO, "rate {rate}");
            assert_eq!(unpriced, vec!["ETH".to_string()], "rate {rate}");
        }
    }

    /// Same symbol, two decimals - a token listed wrong on one chain. The
    /// analytics reader keeps them separable precisely so they are not summed
    /// as if they shared a unit, and this must not undo that.
    #[test]
    fn one_symbol_at_two_decimals_is_scaled_separately_not_summed_raw() {
        let per_asset = assets(&[("USDC", 6, "1000000"), ("USDC", 18, "1000000000000000000")]);
        let (total, unpriced) = total_in(&per_asset, "USD", |_| Some(dec("1")));

        assert_eq!(total, dec("2"), "each group is scaled by its own decimals");
        assert!(unpriced.is_empty());
    }

    /// The database hands back `numeric(78, 0)`, which is wider than this
    /// type. A value that does not fit is dropped rather than saturated: a
    /// saturated figure reads as an enormous volume and refuses a merchant
    /// outright.
    #[test]
    fn a_bucket_too_wide_for_the_type_is_dropped_not_saturated() {
        let buckets = vec![
            data_service::PaymentVolumeBucket {
                day: chrono::NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
                asset_symbol: "USDC".to_string(),
                decimals: 6,
                raw_amount: "1".repeat(40),
                payment_count: 1,
            },
            data_service::PaymentVolumeBucket {
                day: chrono::NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
                asset_symbol: "USDC".to_string(),
                decimals: 6,
                raw_amount: "1000000".to_string(),
                payment_count: 1,
            },
        ];

        let per_asset = collapse_by_asset(buckets);
        assert_eq!(
            per_asset.get(&("USDC".to_string(), 6)),
            Some(&dec("1000000")),
            "the unrepresentable bucket is omitted and the readable one survives"
        );
    }

    /// Days are collapsed; that is the only thing collapsing does.
    #[test]
    fn days_are_summed_into_one_figure_per_asset() {
        let buckets = (1..=3)
            .map(|day| data_service::PaymentVolumeBucket {
                day: chrono::NaiveDate::from_ymd_opt(2026, 9, day).unwrap(),
                asset_symbol: "USDC".to_string(),
                decimals: 6,
                raw_amount: "1000000".to_string(),
                payment_count: 1,
            })
            .collect();

        assert_eq!(
            collapse_by_asset(buckets).get(&("USDC".to_string(), 6)),
            Some(&dec("3000000"))
        );
    }

    #[test]
    fn the_window_ends_now_and_starts_window_days_earlier() {
        let until = at(2026, 9, 20);
        assert_eq!(until - Duration::days(30), at(2026, 8, 21));
    }
}
