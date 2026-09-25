#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

/// A double that answers with a fixed volume and records what it was
/// asked, so the test can check the request survived the JSON boundary.
struct FixedVolume {
    asked: std::sync::Mutex<Vec<(types::UserId, u32, String)>>,
}

#[async_trait::async_trait]
impl super::super::MerchantVolumeReader for FixedVolume {
    async fn merchant_volume(
        &self,
        account_id: types::UserId,
        window_days: u32,
        currency: &str,
    ) -> Result<super::super::MerchantVolume, String> {
        self.asked
            .lock()
            .unwrap()
            .push((account_id, window_days, currency.to_string()));
        Ok(super::super::MerchantVolume {
            volume: "12345.67".to_string(),
            currency: currency.to_string(),
            unpriced_assets: vec!["FOO".to_string()],
        })
    }
}

fn volume_calls(volume: &DeferredVolume) -> PluginCalls {
    let pools = Arc::new(PluginPools::new("postgres://localhost/x".to_string(), 4));
    PluginCalls::new(PluginId::new("cash.random.volume").unwrap(), pools).with_capabilities(
        &DeferredCapabilities {
            issuer: DeferredIssuer::default(),
            volume: volume.clone(),
        },
    )
}

/// The whole point of the capability, through the boundary a plugin
/// actually reaches it by: JSON in, JSON out, and the reader published
/// through the deferred cell rather than held directly.
#[test]
fn a_published_volume_reader_answers_a_plugin_in_its_own_units() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();

    let reader = Arc::new(FixedVolume {
        asked: std::sync::Mutex::new(Vec::new()),
    });
    let volume = DeferredVolume::new();
    assert!(volume.publish(reader.clone()));
    let calls = volume_calls(&volume);

    let account = types::UserId::new();
    let answer = PluginHostCalls::merchant_volume(
        &calls,
        format!(
            r#"{{"account_id":"{}","window_days":30,"currency":"USD"}}"#,
            account.0
        )
        .as_bytes(),
    )
    .unwrap();

    let parsed: serde_json::Value = serde_json::from_slice(&answer).unwrap();
    assert_eq!(parsed["volume"], "12345.67");
    assert_eq!(parsed["currency"], "USD");
    assert_eq!(parsed["unpriced_assets"][0], "FOO");

    let asked = reader.asked.lock().unwrap();
    assert_eq!(
        asked[0],
        (account, 30, "USD".to_string()),
        "the reader must see the account, window and currency the plugin asked for"
    );
}

/// Absent means absent. A plugin that prices on volume and is handed a
/// zero would read it as "this merchant sold nothing" and bill them the
/// bottom bracket forever, which is the one wrong answer that looks
/// entirely normal.
#[test]
fn an_unpublished_volume_reader_is_an_error_and_never_a_zero() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();

    let calls = volume_calls(&DeferredVolume::new());
    let err = PluginHostCalls::merchant_volume(
        &calls,
        format!(
            r#"{{"account_id":"{}","window_days":30}}"#,
            types::UserId::new().0
        )
        .as_bytes(),
    )
    .unwrap_err();

    assert!(err.contains("does not report merchant volume"), "{err}");
}

/// The account is parsed before it reaches a query. A plugin holds the
/// string the host handed it; anything else is a plugin asking about
/// something it made up.
#[test]
fn an_account_id_that_is_not_an_account_id_is_refused() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();

    let reader = Arc::new(FixedVolume {
        asked: std::sync::Mutex::new(Vec::new()),
    });
    let volume = DeferredVolume::new();
    volume.publish(reader.clone());
    let calls = volume_calls(&volume);

    let err = PluginHostCalls::merchant_volume(
        &calls,
        br#"{"account_id":"'; DROP TABLE subscriptions; --","window_days":30}"#,
    )
    .unwrap_err();

    assert!(err.contains("is not an account id"), "{err}");
    assert!(
        reader.asked.lock().unwrap().is_empty(),
        "a request that names no real account must not reach the reader"
    );
}

/// A plugin that omits the currency gets the instance's own unit, not an
/// empty string that would make the answer unreadable.
#[test]
fn an_omitted_currency_falls_back_to_the_instance_unit() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();

    let reader = Arc::new(FixedVolume {
        asked: std::sync::Mutex::new(Vec::new()),
    });
    let volume = DeferredVolume::new();
    volume.publish(reader.clone());
    let calls = volume_calls(&volume);

    PluginHostCalls::merchant_volume(
        &calls,
        format!(
            r#"{{"account_id":"{}","window_days":30,"currency":"  "}}"#,
            types::UserId::new().0
        )
        .as_bytes(),
    )
    .unwrap();

    assert_eq!(reader.asked.lock().unwrap()[0].2, DEFAULT_VOLUME_CURRENCY);
}
