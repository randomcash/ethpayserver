//! Page components for the EVM PayServer client.

mod checkout;
mod dashboard;
mod invoices;
mod not_found;
mod payments;
mod settings;
mod stores;
mod wallets;

/// Trigger a browser file download from in-memory CSV content.
pub(crate) fn trigger_csv_download(content: &str, filename: &str) {
    use wasm_bindgen::JsCast;

    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };

    let parts = js_sys::Array::of1(&wasm_bindgen::JsValue::from_str(content));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type("text/csv;charset=utf-8");

    let Ok(blob) = web_sys::Blob::new_with_str_sequence_and_options(&parts, &opts) else {
        return;
    };
    let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else {
        return;
    };

    if let Ok(a) = document.create_element("a") {
        let _ = a.set_attribute("href", &url);
        let _ = a.set_attribute("download", filename);
        if let Some(el) = a.dyn_ref::<web_sys::HtmlElement>() {
            el.click();
        }
    }

    let _ = web_sys::Url::revoke_object_url(&url);
}

pub use checkout::CheckoutPage;
pub use dashboard::DashboardPage;
pub use invoices::{InvoiceDetailPage, InvoicesPage};
pub use not_found::NotFoundPage;
pub use payments::{PaymentDetailPage, PaymentsPage};
pub use settings::SettingsPage;
pub use stores::{StoreDetailPage, StoresPage};
pub use wallets::{WalletDetailPage, WalletsPage};
