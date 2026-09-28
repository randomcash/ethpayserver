//! Settlement tolerance: how far short of the invoice amount a payment may
//! fall and still settle it.
//!
//! Every comparison is `BigDecimal`. The columns are `NUMERIC(78,18)`, and a
//! fixed-mantissa type would silently round the decision; the tolerance is an
//! explicit, auditable allowance instead.

use std::str::FromStr;

use bigdecimal::{BigDecimal, Zero};
use data_service::{DEFAULT_TOLERANCE_PERCENT, MAX_TOLERANCE_PERCENT};

/// Parse a store-supplied tolerance percentage, refusing anything negative or
/// above [`MAX_TOLERANCE_PERCENT`]. Refuses rather than clamps: a store that
/// thinks it accepts 5% short and actually accepts 1% is worse off than one
/// told no.
pub fn parse_tolerance_percent(raw: &str) -> Result<BigDecimal, &'static str> {
    let value = BigDecimal::from_str(raw.trim()).map_err(|_| "Tolerance is not a number")?;
    if value < BigDecimal::zero() {
        return Err("Tolerance cannot be negative");
    }
    let max = BigDecimal::from_str(MAX_TOLERANCE_PERCENT).map_err(|_| "Invalid ceiling")?;
    if value > max {
        return Err("Tolerance exceeds the maximum allowed");
    }
    Ok(value)
}

/// The tolerance in force for a store, and whether it is the store's own.
pub fn effective_tolerance(
    store_setting: Option<&str>,
) -> Result<(BigDecimal, &'static str), String> {
    match store_setting {
        Some(raw) => parse_tolerance_percent(raw)
            .map(|v| (v, "store"))
            .map_err(|e| format!("Stored tolerance '{raw}' is invalid: {e}")),
        None => BigDecimal::from_str(DEFAULT_TOLERANCE_PERCENT)
            .map(|v| (v, "default"))
            .map_err(|e| e.to_string()),
    }
}

/// Whether `received` settles `expected` given a tolerance in percent of
/// `expected`. Exact payment always does; so does a shortfall up to the
/// tolerance.
pub fn is_fully_paid(
    received: &BigDecimal,
    expected: &BigDecimal,
    tolerance_percent: &BigDecimal,
) -> bool {
    let allowance = expected * tolerance_percent / BigDecimal::from(100);
    received + allowance >= *expected
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn d(s: &str) -> BigDecimal {
        BigDecimal::from_str(s).unwrap()
    }

    #[test]
    fn default_tolerance_is_not_zero() {
        let (value, source) = effective_tolerance(None).unwrap();
        assert!(value > BigDecimal::zero());
        assert_eq!(source, "default");
    }

    #[test]
    fn ceiling_is_refused_not_clamped() {
        assert!(parse_tolerance_percent("1").is_ok());
        assert!(parse_tolerance_percent("1.000000001").is_err());
        assert!(parse_tolerance_percent("50").is_err());
        assert!(parse_tolerance_percent("-0.1").is_err());
        assert!(parse_tolerance_percent("abc").is_err());
    }

    #[test]
    fn boundary_is_inclusive() {
        // 0.1% of 20 is exactly 0.02.
        let tol = d("0.1");
        assert!(is_fully_paid(&d("19.98"), &d("20"), &tol));
        assert!(!is_fully_paid(&d("19.979999999999999999"), &d("20"), &tol));
    }

    #[test]
    fn zero_tolerance_demands_the_full_amount() {
        let zero = d("0");
        assert!(is_fully_paid(&d("20"), &d("20"), &zero));
        assert!(!is_fully_paid(&d("19.999999999999973228"), &d("20"), &zero));
    }
}
