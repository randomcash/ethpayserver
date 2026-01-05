//! API types for ethpayserver.

use serde::{Deserialize, Serialize};

/// Invoice data from the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invoice {
    pub id: String,
    pub store_id: String,
    pub amount: String,
    pub currency: String,
    pub crypto_amount: Option<String>,
    pub crypto_currency: Option<String>,
    pub status: String,
    pub payment_address: Option<String>,
    pub chain_id: Option<u64>,
    pub created_at: String,
    pub expires_at: Option<String>,
}

/// Payment data from the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Payment {
    pub id: String,
    pub invoice_id: String,
    pub tx_hash: String,
    pub amount: String,
    pub currency: String,
    pub from_address: String,
    pub chain_id: u64,
    pub status: String,
    pub confirmations: u32,
    pub created_at: String,
}

/// Store data from the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Store {
    pub id: String,
    pub name: String,
    pub website: Option<String>,
    pub default_currency: String,
    pub enabled_networks: Vec<u64>,
    pub created_at: String,
}

/// Wallet data from the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Wallet {
    pub id: String,
    pub name: String,
    pub address: String,
    pub derivation_path: String,
    pub enabled_chains: Vec<u64>,
    pub created_at: String,
}

/// Dashboard statistics.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DashboardStats {
    pub total_invoices: u64,
    pub pending_invoices: u64,
    pub paid_invoices: u64,
    pub expired_invoices: u64,
    pub total_payments: u64,
    pub total_volume_usd: String,
}

/// Create invoice request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateInvoiceRequest {
    pub store_id: String,
    pub amount: String,
    pub currency: String,
    pub chain_id: Option<u64>,
    pub metadata: Option<serde_json::Value>,
}

/// Create store request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateStoreRequest {
    pub name: String,
    pub website: Option<String>,
    pub default_currency: String,
    pub enabled_networks: Vec<u64>,
}

/// Paginated response wrapper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginatedResponse<T> {
    pub data: Vec<T>,
    pub total: u64,
    pub page: u32,
    pub per_page: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invoice_serialization() {
        let invoice = Invoice {
            id: "inv_001".to_string(),
            store_id: "store_001".to_string(),
            amount: "100.00".to_string(),
            currency: "USD".to_string(),
            crypto_amount: Some("0.05".to_string()),
            crypto_currency: Some("ETH".to_string()),
            status: "pending".to_string(),
            payment_address: Some("0x1234...".to_string()),
            chain_id: Some(1),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            expires_at: None,
        };

        let json = serde_json::to_string(&invoice).unwrap();
        let parsed: Invoice = serde_json::from_str(&json).unwrap();

        assert_eq!(invoice.id, parsed.id);
        assert_eq!(invoice.status, parsed.status);
    }

    #[test]
    fn test_payment_serialization() {
        let payment = Payment {
            id: "pay_001".to_string(),
            invoice_id: "inv_001".to_string(),
            tx_hash: "0xabc...".to_string(),
            amount: "0.05".to_string(),
            currency: "ETH".to_string(),
            from_address: "0x1234...".to_string(),
            chain_id: 1,
            status: "confirmed".to_string(),
            confirmations: 12,
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&payment).unwrap();
        let parsed: Payment = serde_json::from_str(&json).unwrap();

        assert_eq!(payment.id, parsed.id);
        assert_eq!(payment.confirmations, parsed.confirmations);
    }

    #[test]
    fn test_store_serialization() {
        let store = Store {
            id: "store_001".to_string(),
            name: "Test Store".to_string(),
            website: Some("https://example.com".to_string()),
            default_currency: "USD".to_string(),
            enabled_networks: vec![1, 137],
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&store).unwrap();
        let parsed: Store = serde_json::from_str(&json).unwrap();

        assert_eq!(store.id, parsed.id);
        assert_eq!(store.enabled_networks, parsed.enabled_networks);
    }

    #[test]
    fn test_wallet_serialization() {
        let wallet = Wallet {
            id: "wallet_001".to_string(),
            name: "Main Wallet".to_string(),
            address: "0x1234...".to_string(),
            derivation_path: "m/44'/60'/0'/0/0".to_string(),
            enabled_chains: vec![1, 137, 42161],
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&wallet).unwrap();
        let parsed: Wallet = serde_json::from_str(&json).unwrap();

        assert_eq!(wallet.id, parsed.id);
        assert_eq!(wallet.derivation_path, parsed.derivation_path);
    }

    #[test]
    fn test_dashboard_stats_default() {
        let stats = DashboardStats::default();

        assert_eq!(stats.total_invoices, 0);
        assert_eq!(stats.pending_invoices, 0);
        assert_eq!(stats.total_payments, 0);
    }
}
