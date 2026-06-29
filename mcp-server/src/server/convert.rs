//! Currency conversion helpers.
//! (Same logic as server/src/api/invoices.rs)

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

pub(super) fn convert_to_crypto_smallest_unit(
    amount: &str,
    rate: Decimal,
    decimals: u8,
) -> Result<String, String> {
    let parsed: Decimal = amount.parse().map_err(|_| "Invalid amount".to_string())?;
    if parsed <= Decimal::ZERO {
        return Err("Amount must be positive".into());
    }
    let crypto_amount = parsed
        .checked_mul(rate)
        .ok_or_else(|| "Overflow in rate multiplication".to_string())?;
    let smallest = multiply_by_decimals(crypto_amount, decimals)?;
    decimal_to_integer_string(smallest)
}

pub(super) fn convert_human_to_smallest_unit(amount: &str, decimals: u8) -> Result<String, String> {
    let parsed: Decimal = amount.parse().map_err(|_| "Invalid amount".to_string())?;
    if parsed <= Decimal::ZERO {
        return Err("Amount must be positive".into());
    }
    let smallest = multiply_by_decimals(parsed, decimals)?;
    decimal_to_integer_string(smallest)
}

fn multiply_by_decimals(value: Decimal, decimals: u8) -> Result<Decimal, String> {
    let ten = Decimal::from(10);
    let mut multiplier = Decimal::ONE;
    for _ in 0..decimals {
        multiplier = multiplier
            .checked_mul(ten)
            .ok_or_else(|| "Overflow computing multiplier".to_string())?;
    }
    value
        .checked_mul(multiplier)
        .ok_or_else(|| "Overflow in smallest units calculation".to_string())
}

fn decimal_to_integer_string(value: Decimal) -> Result<String, String> {
    let floored = value.floor();
    if floored.is_sign_negative() {
        return Err("Negative amount after conversion".into());
    }
    match floored.to_u128() {
        Some(n) => Ok(n.to_string()),
        None => {
            let normalized = floored.normalize();
            if normalized.scale() > 0 {
                return Err("Unexpected decimal places in conversion result".into());
            }
            let mantissa = normalized.mantissa();
            if mantissa < 0 {
                return Err("Negative amount after conversion".into());
            }
            Ok(mantissa.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_to_crypto_basic() {
        let rate = Decimal::from_str_exact("0.0005").unwrap();
        let result = convert_to_crypto_smallest_unit("100", rate, 18).unwrap();
        assert_eq!(result, "50000000000000000");
    }

    #[test]
    fn test_convert_to_crypto_usdc() {
        let rate = Decimal::ONE;
        let result = convert_to_crypto_smallest_unit("100", rate, 6).unwrap();
        assert_eq!(result, "100000000");
    }

    #[test]
    fn test_convert_human_to_smallest_eth() {
        let result = convert_human_to_smallest_unit("1.5", 18).unwrap();
        assert_eq!(result, "1500000000000000000");
    }

    #[test]
    fn test_convert_rejects_zero() {
        let rate = Decimal::from_str_exact("0.0005").unwrap();
        assert!(convert_to_crypto_smallest_unit("0", rate, 18).is_err());
    }

    #[test]
    fn test_convert_rejects_negative() {
        let rate = Decimal::from_str_exact("0.0005").unwrap();
        assert!(convert_to_crypto_smallest_unit("-100", rate, 18).is_err());
    }

    #[test]
    fn test_convert_rejects_invalid() {
        let rate = Decimal::from_str_exact("0.0005").unwrap();
        assert!(convert_to_crypto_smallest_unit("abc", rate, 18).is_err());
    }

    #[test]
    fn test_convert_human_rejects_negative() {
        assert!(convert_human_to_smallest_unit("-1", 18).is_err());
    }

    #[test]
    fn test_convert_floors_result() {
        let rate = Decimal::from_str_exact("0.0005").unwrap();
        let result = convert_to_crypto_smallest_unit("1.001", rate, 18).unwrap();
        assert_eq!(result, "500500000000000");
    }
}
