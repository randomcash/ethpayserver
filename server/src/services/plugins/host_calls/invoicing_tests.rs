#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

/// A double that answers `invoice_create` and records what it was asked,
/// mirroring `FixedVolume` for capability 3 - the path a billing plugin's
/// `invoice_create` import actually reaches.
struct FixedIssuer {
    own_store: types::StoreId,
    asked: std::sync::Mutex<Vec<super::super::InvoiceCreateRequest>>,
}

#[async_trait::async_trait]
impl super::super::HostInvoiceIssuer for FixedIssuer {
    fn own_store_id(&self) -> types::StoreId {
        self.own_store
    }

    async fn invoice_create(
        &self,
        request: super::super::InvoiceCreateRequest,
    ) -> Result<types::traits::InvoiceData, super::super::InvoiceIssuerError> {
        self.asked.lock().unwrap().push(request.clone());
        Ok(types::traits::InvoiceData {
            id: types::InvoiceId::new(),
            store_id: request.store_id,
            currency: request.asset_symbol.clone(),
            status: types::InvoiceStatus::Pending,
            amount: request.amount.clone(),
            amount_received: "0".to_string(),
            created_at: chrono::Utc::now(),
            expires_at: chrono::Utc::now() + chrono::Duration::seconds(900),
            metadata: request.metadata.clone(),
            customer_email: request.customer_email.clone(),
            extra: None,
        })
    }
}

fn issuer_calls(issuer: &DeferredIssuer) -> PluginCalls {
    let pools = Arc::new(PluginPools::new("postgres://localhost/x".to_string(), 4));
    PluginCalls::new(PluginId::new("cash.random.billing").unwrap(), pools).with_capabilities(
        &DeferredCapabilities {
            issuer: issuer.clone(),
            volume: DeferredVolume::default(),
        },
    )
}

/// The whole point of capability 3, through the boundary a plugin
/// actually reaches it by: JSON in, JSON out, and the issuer published
/// through the deferred cell rather than held directly. This is the
/// path `PluginHostApi::invoice_create` (invoice_issuer.rs) is wired
/// behind - see the doc comment on `PluginHostApi` for the wiring this
/// pins.
#[test]
fn a_published_issuer_answers_a_plugins_invoice_create() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();

    let own_store = types::StoreId::new();
    let reader = Arc::new(FixedIssuer {
        own_store,
        asked: std::sync::Mutex::new(Vec::new()),
    });
    let issuer = DeferredIssuer::new();
    assert!(issuer.publish(reader.clone()));
    let calls = issuer_calls(&issuer);

    let answer = PluginHostCalls::invoice_create(
        &calls,
        br#"{"asset_symbol":"USDC","amount":"49.99","customer_email":"gus@merchant.example"}"#,
    )
    .unwrap();

    let parsed: serde_json::Value = serde_json::from_slice(&answer).unwrap();
    assert_eq!(parsed["currency"], "USDC");
    assert_eq!(parsed["amount"], "49.99");
    assert_eq!(parsed["status"], "pending");
    assert_eq!(
        parsed["checkout_path"],
        format!("/checkout/{}", parsed["invoice_id"].as_str().unwrap())
    );

    let asked = reader.asked.lock().unwrap();
    assert_eq!(
        asked[0].store_id, own_store,
        "the store must come from the issuer's own_store_id, never from the plugin's request"
    );
    assert_eq!(asked[0].asset_symbol, "USDC");
    assert_eq!(asked[0].amount, "49.99");
    assert_eq!(
        asked[0].customer_email,
        Some("gus@merchant.example".to_string())
    );
}

/// Absent means absent, the same property `an_unpublished_volume_reader_is_an_error_and_never_a_zero`
/// pins for capability 6: an instance with no billing store configured
/// must refuse, not silently issue against some default.
#[test]
fn an_unpublished_issuer_is_an_error_and_never_a_default_store() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();

    let calls = issuer_calls(&DeferredIssuer::new());
    let err =
        PluginHostCalls::invoice_create(&calls, br#"{"asset_symbol":"USDC","amount":"49.99"}"#)
            .unwrap_err();

    assert!(err.contains("does not issue invoices"), "{err}");
}
