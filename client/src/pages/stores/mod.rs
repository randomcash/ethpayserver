//! Store management pages - Stripe-inspired design.
//!
//! Uses types from `crate::api::types` which mirror the backend.

mod detail;
mod general_tab;
mod list;
mod payment_methods_tab;
mod settings_tab;
mod webhooks_tab;

pub use detail::StoreDetailPage;
pub use list::StoresPage;

use leptos::prelude::*;

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

/// Helper to get checked state from a checkbox event.
pub(super) fn event_target_checked(ev: &leptos::ev::Event) -> bool {
    use wasm_bindgen::JsCast;
    ev.target()
        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|input| input.checked())
        .unwrap_or(false)
}

// Icons
// ============================================

#[component]
pub(super) fn IconPlus() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="12" y1="5" x2="12" y2="19"></line>
            <line x1="5" y1="12" x2="19" y2="12"></line>
        </svg>
    }
}

#[component]
pub(super) fn IconStore() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M3 9l9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"></path>
            <polyline points="9 22 9 12 15 12 15 22"></polyline>
        </svg>
    }
}

#[component]
pub(super) fn IconGlobe() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="10"></circle>
            <line x1="2" y1="12" x2="22" y2="12"></line>
            <path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"></path>
        </svg>
    }
}

#[component]
pub(super) fn IconCalendar() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="3" y="4" width="18" height="18" rx="2" ry="2"></rect>
            <line x1="16" y1="2" x2="16" y2="6"></line>
            <line x1="8" y1="2" x2="8" y2="6"></line>
            <line x1="3" y1="10" x2="21" y2="10"></line>
        </svg>
    }
}

#[component]
pub(super) fn IconChevronRight() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="9 18 15 12 9 6"></polyline>
        </svg>
    }
}

#[component]
pub(super) fn IconArrowLeft() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="19" y1="12" x2="5" y2="12"></line>
            <polyline points="12 19 5 12 12 5"></polyline>
        </svg>
    }
}

#[component]
pub(super) fn IconArchive() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="21 8 21 21 3 21 3 8"></polyline>
            <rect x="1" y="3" width="22" height="5"></rect>
            <line x1="10" y1="12" x2="14" y2="12"></line>
        </svg>
    }
}

#[component]
pub(super) fn IconMore() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="1"></circle>
            <circle cx="19" cy="12" r="1"></circle>
            <circle cx="5" cy="12" r="1"></circle>
        </svg>
    }
}

#[component]
pub(super) fn IconCopy() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="9" y="9" width="13" height="13" rx="2" ry="2"></rect>
            <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path>
        </svg>
    }
}

#[component]
pub(super) fn IconEye() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"></path>
            <circle cx="12" cy="12" r="3"></circle>
        </svg>
    }
}

#[component]
pub(super) fn IconWebhook() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M18 16.98h-5.99c-1.1 0-1.95.94-2.48 1.9A4 4 0 0 1 2 17c.01-.7.2-1.4.57-2"></path>
            <path d="m6 17 3.13-5.78c.53-.97.1-2.18-.5-3.1a4 4 0 1 1 6.89-4.06"></path>
            <path d="m12 6 3.13 5.73C15.66 12.7 16.9 13 18 13a4 4 0 0 1 0 8"></path>
        </svg>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::chain_name;

    // =========================================================================
    // chain_name
    // =========================================================================

    #[test]
    fn test_chain_name_mainnets() {
        assert_eq!(chain_name(1), "Ethereum");
        assert_eq!(chain_name(137), "Polygon");
        assert_eq!(chain_name(42161), "Arbitrum");
        assert_eq!(chain_name(10), "Optimism");
        assert_eq!(chain_name(8453), "Base");
        assert_eq!(chain_name(56), "BSC");
        assert_eq!(chain_name(43114), "Avalanche");
        assert_eq!(chain_name(324), "zkSync");
        assert_eq!(chain_name(59144), "Linea");
        assert_eq!(chain_name(534352), "Scroll");
        assert_eq!(chain_name(100), "Gnosis");
        assert_eq!(chain_name(250), "Fantom");
    }

    #[test]
    fn test_chain_name_testnet() {
        assert_eq!(chain_name(11155111), "Sepolia");
    }

    #[test]
    fn test_chain_name_unknown() {
        assert_eq!(chain_name(0), "Unknown");
        assert_eq!(chain_name(999999), "Unknown");
    }

    // =========================================================================
    // format_date
    // =========================================================================

    #[test]
    fn test_format_date_iso() {
        assert_eq!(format_date("2024-01-15T10:30:00Z"), "Jan 15, 2024");
        assert_eq!(format_date("2024-06-01T00:00:00Z"), "Jun 01, 2024");
        assert_eq!(format_date("2024-12-25T23:59:59Z"), "Dec 25, 2024");
    }

    #[test]
    fn test_format_date_all_months() {
        assert_eq!(format_date("2024-01-01T00:00:00Z"), "Jan 01, 2024");
        assert_eq!(format_date("2024-02-01T00:00:00Z"), "Feb 01, 2024");
        assert_eq!(format_date("2024-03-01T00:00:00Z"), "Mar 01, 2024");
        assert_eq!(format_date("2024-04-01T00:00:00Z"), "Apr 01, 2024");
        assert_eq!(format_date("2024-05-01T00:00:00Z"), "May 01, 2024");
        assert_eq!(format_date("2024-06-01T00:00:00Z"), "Jun 01, 2024");
        assert_eq!(format_date("2024-07-01T00:00:00Z"), "Jul 01, 2024");
        assert_eq!(format_date("2024-08-01T00:00:00Z"), "Aug 01, 2024");
        assert_eq!(format_date("2024-09-01T00:00:00Z"), "Sep 01, 2024");
        assert_eq!(format_date("2024-10-01T00:00:00Z"), "Oct 01, 2024");
        assert_eq!(format_date("2024-11-01T00:00:00Z"), "Nov 01, 2024");
        assert_eq!(format_date("2024-12-01T00:00:00Z"), "Dec 01, 2024");
    }

    #[test]
    fn test_format_date_date_only() {
        assert_eq!(format_date("2024-01-15"), "Jan 15, 2024");
    }

    #[test]
    fn test_format_date_short_string() {
        // Strings shorter than 10 chars returned as-is
        assert_eq!(format_date("2024"), "2024");
        assert_eq!(format_date(""), "");
    }

    #[test]
    fn test_format_date_malformed() {
        // No dashes — split produces 1 part, returned as-is
        assert_eq!(format_date("2024011500"), "2024011500");
    }
}
