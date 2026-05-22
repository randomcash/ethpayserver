#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use auth::{Store, StoreId, UserId};
use chrono::Utc;
use data_service::StorePaymentMethod;
use uuid::Uuid;

// =========================================================================
// mask_xpub
// =========================================================================

#[test]
fn test_mask_xpub_normal() {
    let xpub = "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz";
    let masked = mask_xpub(xpub);
    assert!(masked.starts_with("xpub6CUG"));
    assert!(masked.contains("..."));
    assert!(masked.ends_with("3fDVmz"));
}

#[test]
fn test_mask_xpub_short() {
    assert_eq!(mask_xpub("short"), "****");
}

#[test]
fn test_mask_xpub_boundary_20() {
    assert_eq!(mask_xpub("12345678901234567890"), "****");
}

#[test]
fn test_mask_xpub_boundary_21() {
    assert_eq!(mask_xpub("123456789012345678901"), "12345678...45678901");
}

// =========================================================================
// StoreResponse conversion
// =========================================================================

#[test]
fn test_store_response_from_store() {
    let now = Utc::now();
    let store = Store {
        id: StoreId(Uuid::nil()),
        name: "Test Store".to_string(),
        website: Some("https://example.com".to_string()),
        owner_id: UserId(Uuid::nil()),
        archived: false,
        created_at: now,
    };

    let response: StoreResponse = store.into();
    assert_eq!(response.id, Uuid::nil());
    assert_eq!(response.name, "Test Store");
    assert_eq!(response.website, Some("https://example.com".to_string()));
    assert_eq!(response.owner_id, Uuid::nil());
    assert!(!response.archived);
    assert_eq!(response.created_at, now);
}

#[test]
fn test_store_response_without_website() {
    let store = Store {
        id: StoreId(Uuid::new_v4()),
        name: "No Website".to_string(),
        website: None,
        owner_id: UserId(Uuid::new_v4()),
        archived: false,
        created_at: Utc::now(),
    };

    let response: StoreResponse = store.into();
    assert!(response.website.is_none());
}

#[test]
fn test_store_response_archived() {
    let store = Store {
        id: StoreId(Uuid::new_v4()),
        name: "Archived".to_string(),
        website: None,
        owner_id: UserId(Uuid::new_v4()),
        archived: true,
        created_at: Utc::now(),
    };

    let response: StoreResponse = store.into();
    assert!(response.archived);
}

// =========================================================================
// PaymentMethodResponse conversion
// =========================================================================

#[test]
fn test_payment_method_response_native() {
    let pm = StorePaymentMethod {
        id: Uuid::nil(),
        store_id: Uuid::nil(),
        chain_id: 1,
        token_address: None,
        asset_symbol: "ETH".to_string(),
        decimals: 18,
        xpub: "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz".to_string(),
        derivation_index: 5,
        enabled: true,
        created_at: Utc::now(),
    };

    let response: PaymentMethodResponse = pm.into();
    assert_eq!(response.chain_id, 1);
    assert_eq!(response.asset_symbol, "ETH");
    assert!(response.token_address.is_none());
    assert_eq!(response.derivation_index, 5);
    assert!(response.enabled);
    assert!(response.xpub_masked.contains("..."));
}

#[test]
fn test_payment_method_response_erc20() {
    let token_addr = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string();
    let pm = StorePaymentMethod {
        id: Uuid::nil(),
        store_id: Uuid::nil(),
        chain_id: 137,
        token_address: Some(token_addr.clone()),
        asset_symbol: "USDC".to_string(),
        decimals: 6,
        xpub: "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz".to_string(),
        derivation_index: 0,
        enabled: false,
        created_at: Utc::now(),
    };

    let response: PaymentMethodResponse = pm.into();
    assert_eq!(response.chain_id, 137);
    assert_eq!(response.asset_symbol, "USDC");
    assert_eq!(response.token_address, Some(token_addr));
    assert!(!response.enabled);
}

// =========================================================================
// Request deserialization
// =========================================================================

#[test]
fn test_create_store_request() {
    let json = r#"{"name": "My Store", "website": "https://mystore.com"}"#;
    let req: CreateStoreRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.name, "My Store");
    assert_eq!(req.website, Some("https://mystore.com".to_string()));
}

#[test]
fn test_create_store_request_without_website() {
    let json = r#"{"name": "My Store"}"#;
    let req: CreateStoreRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.name, "My Store");
    assert!(req.website.is_none());
}

#[test]
fn test_create_store_request_missing_name() {
    let json = r#"{"website": "https://mystore.com"}"#;
    let result = serde_json::from_str::<CreateStoreRequest>(json);
    assert!(result.is_err());
}

#[test]
fn test_update_store_request_partial() {
    let json = r#"{"name": "New Name"}"#;
    let req: UpdateStoreRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.name, Some("New Name".to_string()));
    assert!(req.website.is_none());
}

#[test]
fn test_update_store_request_empty() {
    let json = r#"{}"#;
    let req: UpdateStoreRequest = serde_json::from_str(json).unwrap();
    assert!(req.name.is_none());
    assert!(req.website.is_none());
}

#[test]
fn test_create_payment_method_request() {
    let json = r#"{
        "chain_id": 1,
        "token_address": null,
        "asset_symbol": "ETH",
        "decimals": 18,
        "xpub": "xpub123..."
    }"#;
    let req: CreatePaymentMethodRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.chain_id, 1);
    assert!(req.token_address.is_none());
    assert_eq!(req.asset_symbol, "ETH");
    assert_eq!(req.decimals, 18);
}

#[test]
fn test_update_payment_method_request_partial() {
    let json = r#"{"enabled": false}"#;
    let req: UpdatePaymentMethodRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.enabled, Some(false));
    assert!(req.xpub.is_none());
}

#[test]
fn test_configure_webhook_request_defaults_enabled() {
    let json = r#"{"webhook_url": "https://example.com/hook"}"#;
    let req: ConfigureWebhookRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.webhook_url, "https://example.com/hook");
    assert!(req.enabled);
}

#[test]
fn test_configure_webhook_request_disabled() {
    let json = r#"{"webhook_url": "https://example.com/hook", "enabled": false}"#;
    let req: ConfigureWebhookRequest = serde_json::from_str(json).unwrap();
    assert!(!req.enabled);
}

// =========================================================================
// Response serialization
// =========================================================================

#[test]
fn test_store_response_json() {
    let response = StoreResponse {
        id: Uuid::nil(),
        name: "Test".to_string(),
        website: None,
        owner_id: Uuid::nil(),
        archived: false,
        created_at: Utc::now(),
    };

    let json = serde_json::to_value(&response).unwrap();
    assert_eq!(json["name"], "Test");
    assert_eq!(json["archived"], false);
    assert!(json["website"].is_null());
    assert!(json["id"].is_string());
    assert!(json["created_at"].is_string());
}

#[test]
fn test_member_response_json() {
    let response = MemberResponse {
        user_id: Uuid::nil(),
        store_id: Uuid::nil(),
        role_id: Uuid::nil(),
        role_name: "Owner".to_string(),
        permissions: vec![
            "ethpay.store.canmodifystoresettings".to_string(),
            "ethpay.store.canviewinvoices".to_string(),
        ],
    };

    let json = serde_json::to_value(&response).unwrap();
    assert_eq!(json["role_name"], "Owner");
    assert_eq!(json["permissions"].as_array().unwrap().len(), 2);
}

#[test]
fn test_wallet_response_json() {
    let response = WalletResponse {
        id: Uuid::nil(),
        store_id: Uuid::nil(),
        xpub_masked: "xpub6CUG...3fDVmz".to_string(),
        derivation_index: 42,
        name: Some("Main Wallet".to_string()),
        created_at: Utc::now(),
    };

    let json = serde_json::to_value(&response).unwrap();
    assert_eq!(json["derivation_index"], 42);
    assert_eq!(json["name"], "Main Wallet");
    assert_eq!(json["xpub_masked"], "xpub6CUG...3fDVmz");
}

#[test]
fn test_webhook_response_with_secret() {
    let response = WebhookResponse {
        id: Uuid::nil(),
        store_id: Uuid::nil(),
        webhook_url: "https://example.com/webhook".to_string(),
        webhook_secret: Some("secret123".to_string()),
        enabled: true,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let json = serde_json::to_value(&response).unwrap();
    assert_eq!(json["webhook_url"], "https://example.com/webhook");
    assert_eq!(json["webhook_secret"], "secret123");
    assert!(json["enabled"].as_bool().unwrap());
}

#[test]
fn test_webhook_response_without_secret() {
    let response = WebhookResponse {
        id: Uuid::nil(),
        store_id: Uuid::nil(),
        webhook_url: "https://example.com/webhook".to_string(),
        webhook_secret: None,
        enabled: true,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let json = serde_json::to_value(&response).unwrap();
    assert!(json["webhook_secret"].is_null());
}

// =========================================================================
// list_wallets response shape
// =========================================================================

#[test]
fn test_list_wallets_response_serialization() {
    let wallets = vec![
        WalletResponse {
            id: Uuid::nil(),
            store_id: Uuid::nil(),
            xpub_masked: mask_xpub(
                "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz",
            ),
            derivation_index: 0,
            name: Some("ETH Wallet".to_string()),
            created_at: Utc::now(),
        },
        WalletResponse {
            id: Uuid::new_v4(),
            store_id: Uuid::new_v4(),
            xpub_masked: mask_xpub(
                "xpub6D4BDPcP2GT577Vvch3R8wDkScZWzQzMMUm3PWbmWvVJrZwQY4VUNgqFJPMM3No2dFDFGTsxxpG5uJh7n7epu4trkrX7x7DogT5Uv6fcLW5",
            ),
            derivation_index: 5,
            name: None,
            created_at: Utc::now(),
        },
    ];

    let json = serde_json::to_value(&wallets).unwrap();
    let arr = json.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["name"], "ETH Wallet");
    assert_eq!(arr[0]["derivation_index"], 0);
    assert!(arr[0]["xpub_masked"].as_str().unwrap().contains("..."));
    assert!(arr[1]["name"].is_null());
    assert_eq!(arr[1]["derivation_index"], 5);
}

#[test]
fn test_list_wallets_empty_response() {
    let wallets: Vec<WalletResponse> = vec![];
    let json = serde_json::to_value(&wallets).unwrap();
    assert!(json.as_array().unwrap().is_empty());
}

// =========================================================================
// get_wallet_by_id response
// =========================================================================

#[test]
fn test_wallet_by_id_response_masks_xpub() {
    let xpub = "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz";
    let response = WalletResponse {
        id: Uuid::new_v4(),
        store_id: Uuid::new_v4(),
        xpub_masked: mask_xpub(xpub),
        derivation_index: 7,
        name: Some("Hot Wallet".to_string()),
        created_at: Utc::now(),
    };

    let json = serde_json::to_value(&response).unwrap();
    let masked = json["xpub_masked"].as_str().unwrap();
    assert!(masked.contains("..."));
    assert!(!masked.contains(xpub));
    assert_eq!(json["derivation_index"], 7);
    assert_eq!(json["name"], "Hot Wallet");
}

#[test]
fn test_wallet_by_id_response_without_name() {
    let response = WalletResponse {
        id: Uuid::nil(),
        store_id: Uuid::nil(),
        xpub_masked: "xpub6CUG...3fDVmz".to_string(),
        derivation_index: 0,
        name: None,
        created_at: Utc::now(),
    };

    let json = serde_json::to_value(&response).unwrap();
    assert!(json["name"].is_null());
    assert_eq!(json["derivation_index"], 0);
}

#[test]
fn test_wallet_by_id_response_contains_store_id() {
    let store_id = Uuid::new_v4();
    let response = WalletResponse {
        id: Uuid::new_v4(),
        store_id,
        xpub_masked: "xpub6D4B...cLW5".to_string(),
        derivation_index: 3,
        name: Some("Cold Storage".to_string()),
        created_at: Utc::now(),
    };

    let json = serde_json::to_value(&response).unwrap();
    assert_eq!(json["store_id"].as_str().unwrap(), store_id.to_string());
}

// =========================================================================
// export_wallet_xpub response
// =========================================================================

#[test]
fn test_xpub_export_response_contains_full_xpub() {
    let xpub = "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz";
    let response = WalletXpubResponse {
        id: Uuid::new_v4(),
        store_id: Uuid::new_v4(),
        xpub: xpub.to_string(),
        derivation_index: 5,
        name: Some("Main Wallet".to_string()),
        created_at: Utc::now(),
    };

    let json = serde_json::to_value(&response).unwrap();
    assert_eq!(json["xpub"], xpub);
    assert!(!json["xpub"].as_str().unwrap().contains("..."));
    assert_eq!(json["derivation_index"], 5);
    assert_eq!(json["name"], "Main Wallet");
}

#[test]
fn test_xpub_export_response_without_name() {
    let response = WalletXpubResponse {
        id: Uuid::nil(),
        store_id: Uuid::nil(),
        xpub: "xpub6D4BDPcP2GT577Vvch3R8wDkScZWzQzMMUm3PWbmWvVJrZwQY4VUNgqFJPMM3No2dFDFGTsxxpG5uJh7n7epu4trkrX7x7DogT5Uv6fcLW5".to_string(),
        derivation_index: 0,
        name: None,
        created_at: Utc::now(),
    };

    let json = serde_json::to_value(&response).unwrap();
    assert!(json["name"].is_null());
    assert!(json["xpub"].as_str().unwrap().starts_with("xpub"));
}

// =========================================================================
// list_wallet_addresses response
// =========================================================================

#[test]
fn test_derived_address_entry_serialization() {
    let entry = DerivedAddressEntry {
        address: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
        index: 0,
        derivation_path: "m/44'/60'/0'/0/0".to_string(),
        used: true,
    };

    let json = serde_json::to_value(&entry).unwrap();
    assert_eq!(json["index"], 0);
    assert_eq!(json["derivation_path"], "m/44'/60'/0'/0/0");
    assert_eq!(json["used"], true);
    assert!(json["address"].as_str().unwrap().starts_with("0x"));
}

#[test]
fn test_wallet_addresses_response_marks_used_correctly() {
    let response = WalletAddressesResponse {
        wallet_id: Uuid::new_v4(),
        derivation_index: 3,
        addresses: vec![
            DerivedAddressEntry {
                address: "0xaaa0".to_string(),
                index: 0,
                derivation_path: "m/44'/60'/0'/0/0".to_string(),
                used: true,
            },
            DerivedAddressEntry {
                address: "0xaaa1".to_string(),
                index: 1,
                derivation_path: "m/44'/60'/0'/0/1".to_string(),
                used: true,
            },
            DerivedAddressEntry {
                address: "0xaaa2".to_string(),
                index: 2,
                derivation_path: "m/44'/60'/0'/0/2".to_string(),
                used: true,
            },
            DerivedAddressEntry {
                address: "0xaaa3".to_string(),
                index: 3,
                derivation_path: "m/44'/60'/0'/0/3".to_string(),
                used: false,
            },
            DerivedAddressEntry {
                address: "0xaaa4".to_string(),
                index: 4,
                derivation_path: "m/44'/60'/0'/0/4".to_string(),
                used: false,
            },
        ],
    };

    let json = serde_json::to_value(&response).unwrap();
    assert_eq!(json["derivation_index"], 3);
    let addrs = json["addresses"].as_array().unwrap();
    assert_eq!(addrs.len(), 5);
    // First 3 (index < derivation_index=3) should be used
    assert_eq!(addrs[0]["used"], true);
    assert_eq!(addrs[1]["used"], true);
    assert_eq!(addrs[2]["used"], true);
    // Index 3 and 4 should be unused
    assert_eq!(addrs[3]["used"], false);
    assert_eq!(addrs[4]["used"], false);
}

#[test]
fn test_wallet_addresses_response_empty() {
    let response = WalletAddressesResponse {
        wallet_id: Uuid::new_v4(),
        derivation_index: 0,
        addresses: vec![],
    };

    let json = serde_json::to_value(&response).unwrap();
    assert!(json["addresses"].as_array().unwrap().is_empty());
    assert_eq!(json["derivation_index"], 0);
}

// =========================================================================
// Store Settings validation tests
// =========================================================================

#[test]
fn test_store_settings_response_serialization() {
    let response = StoreSettingsResponse {
        store_id: Uuid::nil(),
        default_chain_id: Some(137),
        default_display_currency: Some("USD".to_string()),
        logo_url: Some("https://example.com/logo.png".to_string()),
        accent_color: Some("#FF5500".to_string()),
        notification_prefs: serde_json::json!({"payment_confirmed": {"webhook": true}}),
        updated_at: "2026-04-20T00:00:00Z".to_string(),
    };
    let json = serde_json::to_value(&response).unwrap();
    assert_eq!(json["default_chain_id"], 137);
    assert_eq!(json["default_display_currency"], "USD");
    assert_eq!(json["logo_url"], "https://example.com/logo.png");
    assert_eq!(json["accent_color"], "#FF5500");
}

#[test]
fn test_store_settings_response_defaults() {
    let response = StoreSettingsResponse {
        store_id: Uuid::nil(),
        default_chain_id: None,
        default_display_currency: None,
        logo_url: None,
        accent_color: None,
        notification_prefs: serde_json::json!({}),
        updated_at: "2026-04-20T00:00:00Z".to_string(),
    };
    let json = serde_json::to_value(&response).unwrap();
    assert!(json["default_chain_id"].is_null());
    assert!(json["logo_url"].is_null());
}

#[test]
fn test_valid_notification_events_list() {
    assert_eq!(VALID_NOTIFICATION_EVENTS.len(), 5);
    assert!(VALID_NOTIFICATION_EVENTS.contains(&"payment_detected"));
    assert!(VALID_NOTIFICATION_EVENTS.contains(&"payment_confirmed"));
    assert!(VALID_NOTIFICATION_EVENTS.contains(&"invoice_expired"));
    assert!(VALID_NOTIFICATION_EVENTS.contains(&"invoice_cancelled"));
    assert!(VALID_NOTIFICATION_EVENTS.contains(&"late_paid"));
}

#[test]
fn test_update_settings_request_deserialization() {
    let json = serde_json::json!({
        "default_chain_id": 1,
        "default_display_currency": "EUR",
        "logo_url": "https://example.com/logo.png",
        "accent_color": "#00FF00",
        "notification_prefs": {"payment_detected": {"webhook": false}}
    });
    let req: UpdateStoreSettingsRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.default_chain_id, Some(1));
    assert_eq!(req.default_display_currency.as_deref(), Some("EUR"));
    assert_eq!(
        req.logo_url.as_deref(),
        Some("https://example.com/logo.png")
    );
    assert_eq!(req.accent_color.as_deref(), Some("#00FF00"));
}

#[test]
fn test_update_settings_request_partial() {
    let json = serde_json::json!({"accent_color": "#AABBCC"});
    let req: UpdateStoreSettingsRequest = serde_json::from_value(json).unwrap();
    assert!(req.default_chain_id.is_none());
    assert!(req.default_display_currency.is_none());
    assert!(req.logo_url.is_none());
    assert_eq!(req.accent_color.as_deref(), Some("#AABBCC"));
    assert!(req.notification_prefs.is_none());
}

// =========================================================================
// Wallet rotation
// =========================================================================

#[test]
fn test_rotate_wallet_request_deserialization() {
    let json = r#"{"xpub": "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz", "reason": "key compromise"}"#;
    let req: RotateWalletRequest = serde_json::from_str(json).unwrap();
    assert!(req.xpub.starts_with("xpub"));
    assert_eq!(req.reason, Some("key compromise".to_string()));
}

#[test]
fn test_rotate_wallet_request_without_reason() {
    let json = r#"{"xpub": "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz"}"#;
    let req: RotateWalletRequest = serde_json::from_str(json).unwrap();
    assert!(req.xpub.starts_with("xpub"));
    assert!(req.reason.is_none());
}

#[test]
fn test_rotate_wallet_request_missing_xpub() {
    let json = r#"{"reason": "test"}"#;
    let result = serde_json::from_str::<RotateWalletRequest>(json);
    assert!(result.is_err());
}

#[test]
fn test_rotate_wallet_response_serialization() {
    let response = RotateWalletResponse {
        store_id: Uuid::nil(),
        new_xpub_masked: "xpub6CUG...3fDVmz".to_string(),
        methods_rotated: 2,
        rotations: vec![
            RotationEntry {
                id: Uuid::new_v4(),
                payment_method_id: Uuid::new_v4(),
                chain_id: 1,
                asset_symbol: "ETH".to_string(),
                previous_xpub_masked: "xpub6D4B...cLW5".to_string(),
                previous_derivation_index: 5,
                rotated_at: Utc::now(),
            },
            RotationEntry {
                id: Uuid::new_v4(),
                payment_method_id: Uuid::new_v4(),
                chain_id: 137,
                asset_symbol: "USDC".to_string(),
                previous_xpub_masked: "xpub6D4B...cLW5".to_string(),
                previous_derivation_index: 12,
                rotated_at: Utc::now(),
            },
        ],
    };

    let json = serde_json::to_value(&response).unwrap();
    assert_eq!(json["methods_rotated"], 2);
    assert_eq!(json["new_xpub_masked"], "xpub6CUG...3fDVmz");
    let rotations = json["rotations"].as_array().unwrap();
    assert_eq!(rotations.len(), 2);
    assert_eq!(rotations[0]["chain_id"], 1);
    assert_eq!(rotations[0]["asset_symbol"], "ETH");
    assert_eq!(rotations[0]["previous_derivation_index"], 5);
    assert_eq!(rotations[1]["chain_id"], 137);
    assert_eq!(rotations[1]["asset_symbol"], "USDC");
}

#[test]
fn test_rotate_wallet_response_empty_rotations() {
    let response = RotateWalletResponse {
        store_id: Uuid::nil(),
        new_xpub_masked: "xpub6CUG...3fDVmz".to_string(),
        methods_rotated: 0,
        rotations: vec![],
    };

    let json = serde_json::to_value(&response).unwrap();
    assert_eq!(json["methods_rotated"], 0);
    assert!(json["rotations"].as_array().unwrap().is_empty());
}
