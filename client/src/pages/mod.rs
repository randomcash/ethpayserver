//! Page components for the EVM PayServer client.

mod checkout;
mod dashboard;
mod invoices;
mod not_found;
mod payments;
mod settings;
mod stores;
mod wallets;

pub use checkout::CheckoutPage;
pub use dashboard::DashboardPage;
pub use invoices::{InvoiceDetailPage, InvoicesPage};
pub use not_found::NotFoundPage;
pub use payments::{PaymentDetailPage, PaymentsPage};
pub use settings::SettingsPage;
pub use stores::{StoreDetailPage, StoresPage};
pub use wallets::{WalletDetailPage, WalletsPage};
