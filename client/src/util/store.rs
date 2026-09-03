//! Store display helpers shared by the invoice and payment list pages.

/// Shorten a store ID for display, or say so when there is nothing to shorten.
///
/// Used as the fallback label in the "All Stores" views (RCS-171) when the
/// server could not resolve a store name: a row still has to say which store it
/// came from, and a bare UUID is better than a blank cell.
///
/// Truncates by chars, not bytes: the ID is a UUID in practice, but a byte
/// slice through a multi-byte char would panic, and a panic in WASM is a blank
/// page rather than one ugly cell.
#[must_use]
pub fn short_store_id(store_id: &str) -> String {
    if store_id.is_empty() {
        return "Unknown store".to_string();
    }
    if store_id.chars().count() > 8 {
        return format!("{}…", store_id.chars().take(8).collect::<String>());
    }
    store_id.to_string()
}

#[cfg(test)]
mod tests {
    use super::short_store_id;

    #[test]
    fn shortens_a_uuid() {
        assert_eq!(
            short_store_id("11111111-2222-3333-4444-555555555555"),
            "11111111\u{2026}"
        );
    }

    #[test]
    fn leaves_short_ids_alone() {
        assert_eq!(short_store_id("short"), "short");
        assert_eq!(short_store_id("12345678"), "12345678");
    }

    #[test]
    fn names_the_absence_of_an_id() {
        assert_eq!(short_store_id(""), "Unknown store");
    }

    #[test]
    fn truncates_on_char_boundaries() {
        // Not a real store ID, but a byte slice here would panic and take the
        // whole page down with it.
        assert_eq!(
            short_store_id("ééééééééé"),
            "\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{2026}"
        );
    }
}
