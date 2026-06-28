//! Formatting and status helpers shared across the payments pages.

use crate::api::Payment;

/// Helper to determine payment status display.
pub(super) fn payment_status(payment: &Payment) -> &'static str {
    if payment.reorged {
        "reorged"
    } else if payment.confirmed_at.is_some() {
        "confirmed"
    } else {
        "pending"
    }
}

/// CSS class for payment status badge.
pub(super) fn payment_status_class(payment: &Payment) -> &'static str {
    if payment.reorged {
        "badge badge-error"
    } else if payment.confirmed_at.is_some() {
        "badge badge-success"
    } else {
        "badge badge-warning"
    }
}

/// Format ISO date string for display.
pub(super) fn format_date(iso: &str) -> String {
    if iso.len() >= 10 {
        let date_part = &iso[..10];
        let parts: Vec<&str> = date_part.split('-').collect();
        if parts.len() == 3 {
            let month = match parts[1] {
                "01" => "Jan",
                "02" => "Feb",
                "03" => "Mar",
                "04" => "Apr",
                "05" => "May",
                "06" => "Jun",
                "07" => "Jul",
                "08" => "Aug",
                "09" => "Sep",
                "10" => "Oct",
                "11" => "Nov",
                "12" => "Dec",
                _ => parts[1],
            };
            return format!("{} {}, {}", month, parts[2], parts[0]);
        }
    }
    iso.to_string()
}

/// Truncate address/hash for display.
pub(super) fn truncate_hash(hash: &str, prefix: usize, suffix: usize) -> String {
    if hash.len() > prefix + suffix + 3 {
        format!("{}...{}", &hash[..prefix], &hash[hash.len() - suffix..])
    } else {
        hash.to_string()
    }
}

/// Format crypto amount from smallest unit to human readable using token decimals.
pub(super) fn format_crypto_amount(amount: &str, decimals: u8) -> String {
    if decimals == 0 {
        return amount.to_string();
    }

    if let Ok(val) = amount.parse::<u128>() {
        let divisor = 10u128.pow(decimals as u32);
        let whole = val / divisor;
        let frac = val % divisor;

        if frac == 0 {
            return whole.to_string();
        }

        let frac_str = format!("{:0width$}", frac, width = decimals as usize);
        let trimmed = frac_str.trim_end_matches('0');
        if trimmed.is_empty() {
            whole.to_string()
        } else {
            format!("{}.{}", whole, trimmed)
        }
    } else {
        amount.to_string()
    }
}
