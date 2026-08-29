//! Stub exchange rate provider.

use async_trait::async_trait;
use chrono::Utc;
use rust_decimal::Decimal;

use rates::{ExchangeRate, RateError, RateProvider};

/// USD → ETH rate for an ETH price of $2000.
///
/// `ExchangeRate` means "1 `from` = `rate` `to`", so $1 buys 0.0005 ETH.
pub const USD_TO_ETH: &str = "0.0005";

/// Serves the pairs it was configured with and reports every other pair as
/// unsupported, so no test can reach the network.
pub struct StubRateProvider {
    rates: Vec<(String, String, Decimal)>,
    /// When set, every lookup fails with a non-`UnsupportedPair` error.
    unavailable: bool,
}

impl StubRateProvider {
    pub fn new() -> Self {
        Self {
            rates: Vec::new(),
            unavailable: false,
        }
    }

    /// A provider that knows the USD/ETH pair — the common case.
    pub fn usd_eth() -> Self {
        Self::new().with_rate("USD", "ETH", USD_TO_ETH)
    }

    /// A provider whose lookups all fail with a transport-style error.
    pub fn unavailable() -> Self {
        Self {
            rates: Vec::new(),
            unavailable: true,
        }
    }

    pub fn with_rate(mut self, from: &str, to: &str, rate: &str) -> Self {
        self.rates.push((
            from.to_uppercase(),
            to.to_uppercase(),
            Decimal::from_str_exact(rate).unwrap(),
        ));
        self
    }
}

#[async_trait]
impl RateProvider for StubRateProvider {
    async fn get_rate(&self, from: &str, to: &str) -> Result<ExchangeRate, RateError> {
        if self.unavailable {
            return Err(RateError::Unavailable);
        }
        let (from_upper, to_upper) = (from.to_uppercase(), to.to_uppercase());
        self.rates
            .iter()
            .find(|(f, t, _)| *f == from_upper && *t == to_upper)
            .map(|(_, _, rate)| ExchangeRate {
                from: from.to_string(),
                to: to.to_string(),
                rate: *rate,
                timestamp: Utc::now(),
            })
            .ok_or_else(|| RateError::UnsupportedPair {
                from: from.to_string(),
                to: to.to_string(),
            })
    }

    fn name(&self) -> &'static str {
        "stub"
    }
}
