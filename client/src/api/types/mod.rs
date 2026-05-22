//! API types for ethpayserver.
//!
//! These types mirror the backend types from payserver-commons/types.

mod admin;
mod api_key;
mod common;
mod invoice;
mod payment;
mod store;
mod user;
mod wallet;

pub use admin::*;
pub use api_key::*;
pub use common::*;
pub use invoice::*;
pub use payment::*;
pub use store::*;
pub use user::*;
pub use wallet::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invoice_status() {
        assert_eq!(InvoiceStatus::Pending.label(), "Pending");
        assert_eq!(InvoiceStatus::Paid.css_class(), "badge badge-success");
        assert!(InvoiceStatus::Paid.is_final());
        assert!(!InvoiceStatus::Pending.is_final());
    }

    #[test]
    fn test_invoice_serialization() {
        let invoice = Invoice {
            id: "inv_001".to_string(),
            amount: "100.00".to_string(),
            currency: "USD".to_string(),
            status: InvoiceStatus::Pending,
            amount_received: "0.00".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            expires_at: "2024-01-02T00:00:00Z".to_string(),
            metadata: None,
            payment_options: vec![],
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
            chain_id: 1,
            invoice_id: "inv_001".to_string(),
            amount: "50000000000000000".to_string(),
            asset_symbol: "ETH".to_string(),
            token_address: None,
            tx_hash: "0xabc123...".to_string(),
            block_number: Some(19000000),
            detected_at: "2024-01-01T00:00:00Z".to_string(),
            confirmed_at: Some("2024-01-01T00:05:00Z".to_string()),
            from_address: Some("0x1234...".to_string()),
            reorged: false,
            decimals: 18,
        };

        let json = serde_json::to_string(&payment).unwrap();
        let parsed: Payment = serde_json::from_str(&json).unwrap();

        assert_eq!(payment.id, parsed.id);
        assert_eq!(payment.chain_id, parsed.chain_id);
        assert_eq!(payment.invoice_id, parsed.invoice_id);
        assert_eq!(payment.confirmed_at, parsed.confirmed_at);
        assert_eq!(payment.decimals, parsed.decimals);
    }

    #[test]
    fn test_store_serialization() {
        let store = Store {
            id: "store_001".to_string(),
            name: "Test Store".to_string(),
            website: Some("https://example.com".to_string()),
            archived: false,
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&store).unwrap();
        let parsed: Store = serde_json::from_str(&json).unwrap();

        assert_eq!(store.id, parsed.id);
        assert_eq!(store.name, parsed.name);
    }

    #[test]
    fn test_wallet_serialization() {
        let wallet = Wallet {
            id: "wallet_001".to_string(),
            store_id: "store_001".to_string(),
            xpub_masked: "xpub6CUG...Ht4QRnxv".to_string(),
            derivation_index: 3,
            name: Some("Main Wallet".to_string()),
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&wallet).unwrap();
        let parsed: Wallet = serde_json::from_str(&json).unwrap();

        assert_eq!(wallet.id, parsed.id);
        assert_eq!(wallet.store_id, parsed.store_id);
        assert_eq!(wallet.xpub_masked, parsed.xpub_masked);
        assert_eq!(wallet.derivation_index, parsed.derivation_index);
        assert_eq!(wallet.name, parsed.name);
    }

    #[test]
    fn test_dashboard_stats_default() {
        let stats = DashboardStats::default();

        assert_eq!(stats.total_invoices, 0);
        assert_eq!(stats.pending_invoices, 0);
        assert_eq!(stats.paid_invoices, 0);
        assert_eq!(stats.expired_invoices, 0);
        assert_eq!(stats.total_payments, 0);
        assert_eq!(stats.total_stores, 0);
    }

    #[test]
    fn test_dashboard_stats_deserialize_from_backend() {
        let json = serde_json::json!({
            "total_invoices": 42,
            "pending_invoices": 5,
            "paid_invoices": 30,
            "expired_invoices": 7,
            "total_payments": 35,
            "total_stores": 3
        });
        let stats: DashboardStats = serde_json::from_value(json).unwrap();
        assert_eq!(stats.total_invoices, 42);
        assert_eq!(stats.pending_invoices, 5);
        assert_eq!(stats.paid_invoices, 30);
        assert_eq!(stats.expired_invoices, 7);
        assert_eq!(stats.total_payments, 35);
        assert_eq!(stats.total_stores, 3);
    }

    #[test]
    fn test_dashboard_stats_roundtrip() {
        let stats = DashboardStats {
            total_invoices: 100,
            pending_invoices: 10,
            paid_invoices: 80,
            expired_invoices: 10,
            total_payments: 95,
            total_stores: 2,
        };
        let json = serde_json::to_value(&stats).unwrap();
        let parsed: DashboardStats = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.total_invoices, 100);
        assert_eq!(parsed.total_stores, 2);
    }

    // =========================================================================
    // Store types
    // =========================================================================

    #[test]
    fn test_store_deserialization_from_backend() {
        // Simulates the JSON the backend actually sends (StoreResponse)
        let json = r#"{
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "name": "My Shop",
            "website": "https://myshop.com",
            "owner_id": "660e8400-e29b-41d4-a716-446655440000",
            "archived": false,
            "created_at": "2024-06-15T10:30:00Z"
        }"#;
        let store: Store = serde_json::from_str(json).unwrap();
        assert_eq!(store.id, "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(store.name, "My Shop");
        assert_eq!(store.website, Some("https://myshop.com".to_string()));
        assert!(!store.archived);
        // owner_id is ignored by client-side Store (extra fields tolerated by serde default)
    }

    #[test]
    fn test_store_deserialization_minimal() {
        // Backend may omit optional fields
        let json = r#"{
            "id": "abc",
            "name": "Bare Store",
            "website": null,
            "created_at": "2024-01-01T00:00:00Z"
        }"#;
        let store: Store = serde_json::from_str(json).unwrap();
        assert_eq!(store.name, "Bare Store");
        assert!(store.website.is_none());
        assert!(!store.archived); // default
    }

    #[test]
    fn test_create_store_request() {
        let req = CreateStoreRequest {
            name: "New Store".to_string(),
            website: Some("https://new.store".to_string()),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["name"], "New Store");
        assert_eq!(json["website"], "https://new.store");
    }

    #[test]
    fn test_create_store_request_without_website() {
        let req = CreateStoreRequest {
            name: "Simple".to_string(),
            website: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["name"], "Simple");
        assert!(json["website"].is_null());
    }

    #[test]
    fn test_update_store_request_serialization() {
        let req = UpdateStoreRequest {
            name: Some("Updated Name".to_string()),
            website: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["name"], "Updated Name");
        assert!(json["website"].is_null());
    }

    // =========================================================================
    // InvoiceStatus coverage
    // =========================================================================

    #[test]
    fn test_invoice_status_all_labels() {
        assert_eq!(InvoiceStatus::Processing.label(), "Processing");
        assert_eq!(InvoiceStatus::PartiallyPaid.label(), "Partially Paid");
        assert_eq!(InvoiceStatus::Expired.label(), "Expired");
        assert_eq!(InvoiceStatus::Cancelled.label(), "Cancelled");
        assert_eq!(InvoiceStatus::Refunded.label(), "Refunded");
        assert_eq!(InvoiceStatus::LatePaid.label(), "Late Paid");
    }

    #[test]
    fn test_invoice_status_all_css_classes() {
        assert_eq!(InvoiceStatus::Pending.css_class(), "badge badge-warning");
        assert_eq!(InvoiceStatus::Processing.css_class(), "badge badge-info");
        assert_eq!(
            InvoiceStatus::PartiallyPaid.css_class(),
            "badge badge-warning"
        );
        assert_eq!(InvoiceStatus::Expired.css_class(), "badge badge-error");
        assert_eq!(InvoiceStatus::Cancelled.css_class(), "badge badge-neutral");
        assert_eq!(InvoiceStatus::Refunded.css_class(), "badge badge-neutral");
        assert_eq!(InvoiceStatus::LatePaid.css_class(), "badge badge-info");
    }

    #[test]
    fn test_invoice_status_is_final() {
        // Final statuses
        assert!(InvoiceStatus::Paid.is_final());
        assert!(InvoiceStatus::Expired.is_final());
        assert!(InvoiceStatus::Cancelled.is_final());
        assert!(InvoiceStatus::Refunded.is_final());
        assert!(InvoiceStatus::LatePaid.is_final());
        // Non-final
        assert!(!InvoiceStatus::Pending.is_final());
        assert!(!InvoiceStatus::Processing.is_final());
        assert!(!InvoiceStatus::PartiallyPaid.is_final());
    }

    #[test]
    fn test_invoice_status_default() {
        // Verify that deserializing an Invoice without a status field
        // defaults to Pending (via the serde default function).
        let json = r#"{
            "id": "inv-default",
            "currency": "USD",
            "amount": "10",
            "amount_received": "0",
            "created_at": "2024-01-01T00:00:00Z",
            "expires_at": "2024-01-02T00:00:00Z",
            "metadata": null
        }"#;
        let invoice: Invoice = serde_json::from_str(json).unwrap();
        assert_eq!(invoice.status, InvoiceStatus::Pending);
    }

    #[test]
    fn test_invoice_status_serde_roundtrip() {
        // snake_case serialization
        let json = serde_json::to_string(&InvoiceStatus::PartiallyPaid).unwrap();
        assert_eq!(json, "\"partially_paid\"");
        let parsed: InvoiceStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, InvoiceStatus::PartiallyPaid);
    }

    // =========================================================================
    // InvoiceStatusResponse
    // =========================================================================

    #[test]
    fn test_invoice_status_response_from_backend() {
        let json = r#"{
            "id": "inv-1",
            "status": "paid",
            "amount": "100.00",
            "amount_received": "100.00",
            "currency": "USD",
            "expires_at": "2024-01-02T00:00:00Z",
            "payment_count": 1,
            "confirmed_count": 1,
            "is_paid": true,
            "is_expired": false,
            "payment_options": [],
            "payments": [
                {
                    "id": "pay-1",
                    "chain_id": 1,
                    "invoice_id": "inv-1",
                    "tx_hash": "0xabc123",
                    "amount": "50000000000000000",
                    "asset_symbol": "ETH",
                    "token_address": null,
                    "block_number": 19000000,
                    "from_address": "0x1234",
                    "detected_at": "2024-01-01T10:00:00Z",
                    "confirmed_at": "2024-01-01T10:05:00Z",
                    "reorged": false
                }
            ]
        }"#;
        let resp: InvoiceStatusResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.id, "inv-1");
        assert_eq!(resp.status, InvoiceStatus::Paid);
        assert!(resp.is_paid);
        assert_eq!(resp.payments.len(), 1);
        assert_eq!(resp.payments[0].tx_hash, "0xabc123");
    }

    // =========================================================================
    // Payment method & webhook
    // =========================================================================

    #[test]
    fn test_store_payment_method_serialization() {
        let pm = StorePaymentMethod {
            id: "pm_001".to_string(),
            store_id: "store_001".to_string(),
            chain_id: 1,
            token_address: None,
            asset_symbol: "ETH".to_string(),
            xpub_masked: "xpub12...pub123".to_string(),
            derivation_index: 0,
            enabled: true,
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&pm).unwrap();
        let parsed: StorePaymentMethod = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.chain_id, 1);
        assert_eq!(parsed.asset_symbol, "ETH");
        assert!(parsed.enabled);
        assert!(parsed.token_address.is_none());
    }

    #[test]
    fn test_store_payment_method_erc20() {
        let pm = StorePaymentMethod {
            id: "pm_002".to_string(),
            store_id: "store_001".to_string(),
            chain_id: 137,
            token_address: Some("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string()),
            asset_symbol: "USDC".to_string(),
            xpub_masked: "xpub45...pub456".to_string(),
            derivation_index: 5,
            enabled: false,
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&pm).unwrap();
        let parsed: StorePaymentMethod = serde_json::from_str(&json).unwrap();
        assert!(!parsed.enabled);
        assert!(parsed.token_address.is_some());
    }

    #[test]
    fn test_store_payment_method_enabled_default() {
        // When 'enabled' is missing from JSON, it should default to true
        let json = r#"{
            "id": "pm_003",
            "store_id": "s1",
            "chain_id": 1,
            "token_address": null,
            "asset_symbol": "ETH",
            "xpub_masked": "xpub...pub",
            "derivation_index": 0,
            "created_at": "2024-01-01T00:00:00Z"
        }"#;
        let pm: StorePaymentMethod = serde_json::from_str(json).unwrap();
        assert!(pm.enabled);
    }

    #[test]
    fn test_store_payment_method_from_backend_json() {
        // Simulates the actual PaymentMethodResponse JSON from the backend
        let json = r#"{
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "store_id": "660e8400-e29b-41d4-a716-446655440000",
            "chain_id": 11155111,
            "token_address": null,
            "asset_symbol": "ETH",
            "xpub_masked": "xpub6CUG...Ht4QRnxv",
            "derivation_index": 3,
            "enabled": true,
            "created_at": "2024-06-15T10:30:00Z"
        }"#;
        let pm: StorePaymentMethod = serde_json::from_str(json).unwrap();
        assert_eq!(pm.id, "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(pm.chain_id, 11155111);
        assert_eq!(pm.asset_symbol, "ETH");
        assert_eq!(pm.xpub_masked, "xpub6CUG...Ht4QRnxv");
        assert_eq!(pm.derivation_index, 3);
        assert!(pm.enabled);
        assert!(pm.token_address.is_none());
    }

    #[test]
    fn test_store_payment_method_from_backend_erc20_json() {
        // Backend response for an ERC20 payment method
        let json = r#"{
            "id": "770e8400-e29b-41d4-a716-446655440000",
            "store_id": "660e8400-e29b-41d4-a716-446655440000",
            "chain_id": 1,
            "token_address": "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
            "asset_symbol": "USDC",
            "xpub_masked": "xpub6D4B...9kW3F2Rq",
            "derivation_index": 0,
            "enabled": false,
            "created_at": "2024-06-15T10:30:00Z"
        }"#;
        let pm: StorePaymentMethod = serde_json::from_str(json).unwrap();
        assert_eq!(pm.chain_id, 1);
        assert_eq!(pm.asset_symbol, "USDC");
        assert_eq!(
            pm.token_address.as_deref(),
            Some("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48")
        );
        assert!(!pm.enabled);
    }

    #[test]
    fn test_create_payment_method_request() {
        let req = CreatePaymentMethodRequest {
            chain_id: 1,
            token_address: None,
            asset_symbol: "ETH".to_string(),
            decimals: 18,
            xpub: "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz".to_string(),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["chain_id"], 1);
        assert!(json["token_address"].is_null());
        assert_eq!(json["asset_symbol"], "ETH");
        assert_eq!(json["decimals"], 18);
        assert!(json["xpub"].as_str().unwrap().starts_with("xpub"));
    }

    #[test]
    fn test_create_payment_method_request_erc20() {
        let req = CreatePaymentMethodRequest {
            chain_id: 137,
            token_address: Some("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string()),
            asset_symbol: "USDC".to_string(),
            decimals: 6,
            xpub: "xpub123...".to_string(),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["chain_id"], 137);
        assert_eq!(
            json["token_address"],
            "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
        );
        assert_eq!(json["decimals"], 6);
    }

    #[test]
    fn test_update_payment_method_request_toggle_enabled() {
        let req = UpdatePaymentMethodRequest {
            enabled: Some(false),
            xpub: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["enabled"], false);
        assert!(json["xpub"].is_null());
    }

    #[test]
    fn test_update_payment_method_request_change_xpub() {
        let req = UpdatePaymentMethodRequest {
            enabled: None,
            xpub: Some("xpub6NEW...".to_string()),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert!(json["enabled"].is_null());
        assert_eq!(json["xpub"], "xpub6NEW...");
    }

    #[test]
    fn test_update_payment_method_request_both_fields() {
        let req = UpdatePaymentMethodRequest {
            enabled: Some(true),
            xpub: Some("xpub6ABC...".to_string()),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["enabled"], true);
        assert_eq!(json["xpub"], "xpub6ABC...");
    }

    #[test]
    fn test_store_webhook_serialization() {
        let wh = StoreWebhook {
            id: "wh_001".to_string(),
            store_id: "store_001".to_string(),
            webhook_url: "https://example.com/hook".to_string(),
            webhook_secret: Some("secret123".to_string()),
            enabled: true,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&wh).unwrap();
        let parsed: StoreWebhook = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.webhook_url, "https://example.com/hook");
        assert_eq!(parsed.webhook_secret, Some("secret123".to_string()));
        assert!(parsed.enabled);
    }

    #[test]
    fn test_store_webhook_from_backend_get() {
        // Backend GET returns webhook_secret as null
        let json = r#"{
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "store_id": "660e8400-e29b-41d4-a716-446655440000",
            "webhook_url": "https://example.com/hook",
            "webhook_secret": null,
            "enabled": true,
            "created_at": "2024-06-15T10:30:00Z",
            "updated_at": "2024-06-15T10:30:00Z"
        }"#;
        let wh: StoreWebhook = serde_json::from_str(json).unwrap();
        assert!(wh.webhook_secret.is_none());
        assert!(wh.enabled);
    }

    #[test]
    fn test_store_webhook_from_backend_put() {
        // Backend PUT returns webhook_secret with the new secret
        let json = r#"{
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "store_id": "660e8400-e29b-41d4-a716-446655440000",
            "webhook_url": "https://example.com/hook",
            "webhook_secret": "whsec_abc123",
            "enabled": true,
            "created_at": "2024-06-15T10:30:00Z",
            "updated_at": "2024-06-15T10:30:00Z"
        }"#;
        let wh: StoreWebhook = serde_json::from_str(json).unwrap();
        assert_eq!(wh.webhook_secret, Some("whsec_abc123".to_string()));
    }

    #[test]
    fn test_store_webhook_enabled_default() {
        let json = r#"{
            "id": "wh_002",
            "store_id": "s1",
            "webhook_url": "https://example.com/hook",
            "webhook_secret": null,
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z"
        }"#;
        let wh: StoreWebhook = serde_json::from_str(json).unwrap();
        assert!(wh.enabled);
    }

    // =========================================================================
    // UpdateWebhookRequest
    // =========================================================================

    #[test]
    fn test_update_webhook_request_serialization() {
        let req = UpdateWebhookRequest {
            webhook_url: "https://example.com/hook".to_string(),
            enabled: true,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["webhook_url"], "https://example.com/hook");
        assert_eq!(json["enabled"], true);
    }

    #[test]
    fn test_update_webhook_request_disabled() {
        let req = UpdateWebhookRequest {
            webhook_url: "http://localhost:1234".to_string(),
            enabled: false,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["webhook_url"], "http://localhost:1234");
        assert_eq!(json["enabled"], false);
    }

    #[test]
    fn test_update_webhook_request_roundtrip() {
        let req = UpdateWebhookRequest {
            webhook_url: "https://api.example.com/webhooks/payments".to_string(),
            enabled: true,
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: UpdateWebhookRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.webhook_url, req.webhook_url);
        assert_eq!(parsed.enabled, req.enabled);
    }

    // =========================================================================
    // Paginated response
    // =========================================================================

    #[test]
    fn test_paginated_response() {
        let json = r#"{
            "data": [
                {"id": "inv_1", "currency": "USD", "status": "pending", "amount": "100", "amount_received": "0", "created_at": "2024-01-01T00:00:00Z", "expires_at": "2024-01-02T00:00:00Z", "metadata": null}
            ],
            "total": 50,
            "page": 1,
            "per_page": 10
        }"#;
        let resp: PaginatedResponse<Invoice> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.data.len(), 1);
        assert_eq!(resp.total, 50);
        assert_eq!(resp.page, 1);
        assert_eq!(resp.per_page, 10);
        assert_eq!(resp.data[0].id, "inv_1");
    }

    // =========================================================================
    // Create invoice request
    // =========================================================================

    // =========================================================================
    // InvoiceListResponse (backend response for GET /invoices)
    // =========================================================================

    #[test]
    fn test_invoice_list_response_from_backend() {
        let json = r#"{
            "total": 42,
            "invoices": [
                {
                    "id": "550e8400-e29b-41d4-a716-446655440000",
                    "currency": "USD",
                    "status": "paid",
                    "amount": "100.00",
                    "amount_received": "100.00",
                    "created_at": "2024-06-15T10:30:00Z",
                    "expires_at": "2024-06-16T10:30:00Z",
                    "metadata": {"order_id": "ORD-123"},
                    "payment_options": []
                },
                {
                    "id": "660e8400-e29b-41d4-a716-446655440000",
                    "currency": "ETH",
                    "status": "pending",
                    "amount": "0.5",
                    "amount_received": "0",
                    "created_at": "2024-06-15T11:00:00Z",
                    "expires_at": "2024-06-15T11:15:00Z",
                    "metadata": null,
                    "payment_options": []
                }
            ]
        }"#;
        let resp: InvoiceListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.total, 42);
        assert_eq!(resp.invoices.len(), 2);
        assert_eq!(resp.invoices[0].id, "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(resp.invoices[0].status, InvoiceStatus::Paid);
        assert_eq!(resp.invoices[0].amount, "100.00");
        assert_eq!(resp.invoices[1].status, InvoiceStatus::Pending);
    }

    #[test]
    fn test_invoice_list_response_empty() {
        let json = r#"{"total": 0, "invoices": []}"#;
        let resp: InvoiceListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.total, 0);
        assert!(resp.invoices.is_empty());
    }

    #[test]
    fn test_invoice_from_backend() {
        let json = r#"{
            "id": "inv-1",
            "currency": "USD",
            "status": "expired",
            "amount": "50.00",
            "amount_received": "0",
            "created_at": "2024-01-01T00:00:00Z",
            "expires_at": "2024-01-01T01:00:00Z",
            "metadata": null,
            "payment_options": []
        }"#;
        let invoice: Invoice = serde_json::from_str(json).unwrap();
        assert_eq!(invoice.id, "inv-1");
        assert_eq!(invoice.status, InvoiceStatus::Expired);
        assert!(invoice.payment_options.is_empty());
    }

    #[test]
    fn test_invoice_with_payment_options() {
        let json = r#"{
            "id": "inv-2",
            "currency": "USD",
            "status": "pending",
            "amount": "100.00",
            "amount_received": "0",
            "created_at": "2024-01-01T00:00:00Z",
            "expires_at": "2024-01-01T01:00:00Z",
            "metadata": null,
            "payment_options": [
                {
                    "id": "po-1",
                    "payment_method_id": "ETH-1",
                    "chain_id": 1,
                    "asset_symbol": "ETH",
                    "token_address": null,
                    "decimals": 18,
                    "payment_address": "0xabc123",
                    "amount": "28000000000000000",
                    "rate": "0.00028",
                    "is_active": true
                }
            ]
        }"#;
        let invoice: Invoice = serde_json::from_str(json).unwrap();
        assert_eq!(invoice.payment_options.len(), 1);
        assert_eq!(invoice.payment_options[0].chain_id, 1);
        assert_eq!(invoice.payment_options[0].asset_symbol, "ETH");
    }

    #[test]
    fn test_invoice_list_response_roundtrip() {
        let resp = InvoiceListResponse {
            total: 1,
            invoices: vec![Invoice {
                id: "inv-rt".to_string(),
                currency: "USD".to_string(),
                status: InvoiceStatus::Paid,
                amount: "25.00".to_string(),
                amount_received: "25.00".to_string(),
                created_at: "2024-01-01T00:00:00Z".to_string(),
                expires_at: "2024-01-02T00:00:00Z".to_string(),
                metadata: None,
                payment_options: vec![],
            }],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: InvoiceListResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.total, 1);
        assert_eq!(parsed.invoices[0].id, "inv-rt");
    }

    // =========================================================================
    // Create invoice request
    // =========================================================================

    #[test]
    fn test_create_invoice_request() {
        let req = CreateInvoiceRequest {
            store_id: "store_001".to_string(),
            amount: "99.99".to_string(),
            currency: "USD".to_string(),
            expiration_seconds: Some(1800),
            metadata: None,
            customer_email: None,
            webhook_url: None,
            redirect_url: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["store_id"], "store_001");
        assert_eq!(json["amount"], "99.99");
        assert_eq!(json["currency"], "USD");
        assert_eq!(json["expiration_seconds"], 1800);
        // skip_serializing_if = None fields should be absent
        assert!(json.get("metadata").is_none());
        assert!(json.get("webhook_url").is_none());
    }

    #[test]
    fn test_create_invoice_request_minimal() {
        let req = CreateInvoiceRequest {
            store_id: "s1".to_string(),
            amount: "10".to_string(),
            currency: "ETH".to_string(),
            expiration_seconds: None,
            metadata: None,
            customer_email: None,
            webhook_url: None,
            redirect_url: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["store_id"], "s1");
        // Optional fields should not be present in JSON
        assert!(json.get("expiration_seconds").is_none());
        assert!(json.get("metadata").is_none());
    }

    #[test]
    fn test_user_role_default_is_user() {
        assert_eq!(UserRole::default(), UserRole::User);
    }

    #[test]
    fn test_user_role_labels() {
        assert_eq!(UserRole::ServerAdmin.label(), "Server Admin");
        assert_eq!(UserRole::User.label(), "User");
    }

    #[test]
    fn test_user_role_is_admin() {
        assert!(UserRole::ServerAdmin.is_admin());
        assert!(!UserRole::User.is_admin());
    }

    #[test]
    fn test_user_role_serde_roundtrip() {
        let admin_json = serde_json::to_value(UserRole::ServerAdmin).unwrap();
        assert_eq!(admin_json, serde_json::json!("server_admin"));

        let user_json = serde_json::to_value(UserRole::User).unwrap();
        assert_eq!(user_json, serde_json::json!("user"));

        let parsed: UserRole = serde_json::from_str("\"server_admin\"").unwrap();
        assert_eq!(parsed, UserRole::ServerAdmin);

        let parsed: UserRole = serde_json::from_str("\"user\"").unwrap();
        assert_eq!(parsed, UserRole::User);
    }

    #[test]
    fn test_user_info_deserialize_full() {
        let json = serde_json::json!({
            "id": "usr_123",
            "email": "alice@example.com",
            "primary_wallet_address": "0xabc",
            "created_at": "2026-01-01T00:00:00Z",
            "last_login_at": "2026-04-04T12:00:00Z",
            "role": "server_admin"
        });
        let user: UserInfo = serde_json::from_value(json).unwrap();
        assert_eq!(user.id, "usr_123");
        assert_eq!(user.email.as_deref(), Some("alice@example.com"));
        assert_eq!(user.primary_wallet_address.as_deref(), Some("0xabc"));
        assert_eq!(user.last_login_at.as_deref(), Some("2026-04-04T12:00:00Z"));
        assert!(user.role.is_admin());
    }

    #[test]
    fn test_user_info_deserialize_minimal() {
        let json = serde_json::json!({
            "id": "usr_456",
            "email": null,
            "primary_wallet_address": null,
            "created_at": "2026-03-15T10:00:00Z",
            "last_login_at": null,
            "role": "user"
        });
        let user: UserInfo = serde_json::from_value(json).unwrap();
        assert_eq!(user.id, "usr_456");
        assert!(user.email.is_none());
        assert!(user.primary_wallet_address.is_none());
        assert!(user.last_login_at.is_none());
        assert!(!user.role.is_admin());
    }
}
