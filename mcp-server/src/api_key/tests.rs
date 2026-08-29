//! Tests for the MCP API key authentication flow.
//!
//! The MCP server has no per-request auth: it authenticates the raw key once,
//! at start-up, and the resulting `(UserId, Vec<StoreId>)` is the session scope
//! every later tool call is checked against. These tests cover that gate.

use chrono::{Duration, Utc};
use uuid::Uuid;

use auth::UserId;

use crate::testkit::{RAW_KEY, StubAuthRepo, test_api_key, test_store};

use super::{hash_api_key, validate_api_key};

// =========================================================================
// hash_api_key
// =========================================================================

#[test]
fn hash_api_key_is_sha256_hex() {
    // Known SHA-256 vector for the ASCII string "abc".
    assert_eq!(
        hash_api_key("abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn hash_api_key_is_deterministic_and_key_specific() {
    assert_eq!(hash_api_key(RAW_KEY), hash_api_key(RAW_KEY));
    assert_ne!(hash_api_key(RAW_KEY), hash_api_key("ak_live_abc124"));
}

// =========================================================================
// validate_api_key
// =========================================================================

#[tokio::test]
async fn validate_api_key_returns_owner_and_store_scope() {
    let user_id = UserId(Uuid::new_v4());
    let store = test_store(user_id);
    let store_id = store.id;
    let repo = StubAuthRepo::with_key(test_api_key(user_id)).with_store(store);

    let (resolved_user, store_ids) = validate_api_key(&repo, RAW_KEY).await.unwrap();

    assert_eq!(resolved_user, user_id);
    assert_eq!(store_ids, vec![store_id]);
}

#[tokio::test]
async fn validate_api_key_looks_the_key_up_by_hash_not_plaintext() {
    let user_id = UserId(Uuid::new_v4());
    let repo = StubAuthRepo::with_key(test_api_key(user_id));

    // The stub only matches on the stored SHA-256 hash, so a lookup that
    // passed the raw key through would find nothing.
    assert!(validate_api_key(&repo, RAW_KEY).await.is_ok());
}

#[tokio::test]
async fn validate_api_key_rejects_unknown_key() {
    let repo = StubAuthRepo::with_key(test_api_key(UserId(Uuid::new_v4())));

    let err = validate_api_key(&repo, "ak_live_wrong").await.unwrap_err();

    assert!(err.to_string().contains("Invalid API key"), "got: {err}");
}

#[tokio::test]
async fn validate_api_key_rejects_deactivated_key() {
    let user_id = UserId(Uuid::new_v4());
    let mut key = test_api_key(user_id);
    key.is_active = false;
    let repo = StubAuthRepo::with_key(key).with_store(test_store(user_id));

    let err = validate_api_key(&repo, RAW_KEY).await.unwrap_err();

    assert!(err.to_string().contains("deactivated"), "got: {err}");
}

#[tokio::test]
async fn validate_api_key_rejects_expired_key() {
    let user_id = UserId(Uuid::new_v4());
    let mut key = test_api_key(user_id);
    key.expires_at = Some(Utc::now() - Duration::seconds(1));
    let repo = StubAuthRepo::with_key(key).with_store(test_store(user_id));

    let err = validate_api_key(&repo, RAW_KEY).await.unwrap_err();

    assert!(err.to_string().contains("expired"), "got: {err}");
}

#[tokio::test]
async fn validate_api_key_accepts_key_expiring_in_the_future() {
    let user_id = UserId(Uuid::new_v4());
    let mut key = test_api_key(user_id);
    key.expires_at = Some(Utc::now() + Duration::hours(1));
    let repo = StubAuthRepo::with_key(key);

    assert!(validate_api_key(&repo, RAW_KEY).await.is_ok());
}

#[tokio::test]
async fn validate_api_key_records_last_used() {
    let user_id = UserId(Uuid::new_v4());
    let key = test_api_key(user_id);
    let key_id = key.id;
    let repo = StubAuthRepo::with_key(key);

    validate_api_key(&repo, RAW_KEY).await.unwrap();

    assert_eq!(repo.last_used_calls(), vec![key_id]);
}

#[tokio::test]
async fn validate_api_key_does_not_record_last_used_for_a_rejected_key() {
    let user_id = UserId(Uuid::new_v4());
    let mut key = test_api_key(user_id);
    key.is_active = false;
    let repo = StubAuthRepo::with_key(key);

    assert!(validate_api_key(&repo, RAW_KEY).await.is_err());
    assert!(repo.last_used_calls().is_empty());
}

#[tokio::test]
async fn validate_api_key_surfaces_repository_errors() {
    let repo = StubAuthRepo::failing_lookup();

    let err = validate_api_key(&repo, RAW_KEY).await.unwrap_err();

    assert!(
        err.to_string()
            .contains("Database error looking up API key"),
        "got: {err}"
    );
}

#[tokio::test]
async fn validate_api_key_yields_empty_scope_when_the_owner_has_no_stores() {
    let user_id = UserId(Uuid::new_v4());
    // Store owned by a different user — not in this key's scope.
    let repo = StubAuthRepo::with_key(test_api_key(user_id))
        .with_store(test_store(UserId(Uuid::new_v4())));

    let (_, store_ids) = validate_api_key(&repo, RAW_KEY).await.unwrap();

    assert!(store_ids.is_empty());
}
