//! The scope-less schema means `cross_tenant_reads` and `scope_parity` are
//! the whole story for "does a key ever reach more than its owner's data" -
//! but there is a second way a key can carry more than it should: the
//! active/expiry check that gates the hash lookup silently not applying.
//! These two hit that mechanism directly, going through
//! `AuthenticatedUser::from_request_parts` (not the `authenticate_via_bearer`
//! helper, which unwraps and would panic on the rejection under test)
//! against a key whose row is revoked, or expired, exactly the way an
//! operator revocation or a client-set TTL would leave it.

use axum::extract::FromRequestParts;
use axum::http::{Request as HttpRequest, StatusCode};
use chrono::Utc;
use uuid::Uuid;

use auth::{ApiKey, ApiKeyId, ApiKeyRepository, UserId};
use server::api::AuthenticatedUser;

use crate::support::{app_state, seed_tenant, service, sha256_hex};

#[tokio::test]
#[ignore]
async fn a_revoked_api_key_no_longer_authenticates() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;
    let key = ApiKeyRepository::get_api_key_by_hash(&pg, &sha256_hex(&a.api_key_raw))
        .await
        .expect("look up the seeded key")
        .expect("seeded key exists");
    ApiKeyRepository::revoke_api_key(&pg, key.id)
        .await
        .expect("revoke it the same way an operator revocation would");
    let state = app_state(std::sync::Arc::new(pg));

    let request = HttpRequest::builder()
        .header("authorization", format!("Bearer {}", a.api_key_raw))
        .body(())
        .expect("build request");
    let (mut parts, ()) = request.into_parts();
    let result = AuthenticatedUser::from_request_parts(&mut parts, &state).await;
    assert_eq!(
        result.err().map(|(status, _)| status),
        Some(StatusCode::UNAUTHORIZED),
        "a revoked key's hash still matches its row; the active flag, not the \
         hash lookup, is what must refuse it"
    );
}

#[tokio::test]
#[ignore]
async fn an_expired_api_key_no_longer_authenticates() {
    let Some(pg) = service().await else {
        return;
    };
    let a = seed_tenant(&pg, "a").await;

    let expired_raw = format!("ak_test_{}", Uuid::new_v4());
    ApiKeyRepository::create_api_key(
        &pg,
        &ApiKey {
            id: ApiKeyId::new(),
            user_id: UserId(a.user_id),
            name: "expired cross-tenant test key".to_string(),
            key_hash: sha256_hex(&expired_raw),
            key_prefix: "ak_test_****".to_string(),
            is_active: true,
            created_at: Utc::now() - chrono::Duration::hours(2),
            last_used_at: None,
            expires_at: Some(Utc::now() - chrono::Duration::hours(1)),
        },
    )
    .await
    .expect("seed an already-expired api key");
    let state = app_state(std::sync::Arc::new(pg));

    let request = HttpRequest::builder()
        .header("authorization", format!("Bearer {expired_raw}"))
        .body(())
        .expect("build request");
    let (mut parts, ()) = request.into_parts();
    let result = AuthenticatedUser::from_request_parts(&mut parts, &state).await;
    assert_eq!(
        result.err().map(|(status, _)| status),
        Some(StatusCode::UNAUTHORIZED),
        "an expired key's hash still matches its row; the expiry check, not \
         the hash lookup, is what must refuse it"
    );
}
