#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The account-standing receiver, driven through the production router with a
//! real `Authorization: Bearer` key against a real Postgres.
//!
//! Every credential here is exercised as itself: a key without the push scope
//! is refused as that key, not as an admin, and an unrestricted admin key is
//! refused too, because the sender must hold a credential good for nothing
//! else.
//!
//! Needs `DATABASE_URL`; skips when unset, like the other ignored integration
//! tests, and runs in CI's `--run-ignored` step.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use data_service::{AccountStandingStore, PgDataService};
use rates::NoOpRateProvider;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use uuid::Uuid;

use auth::{AuthConfig, AuthService};
use server::services::RedisEVMMonitor;
use server::state::PgAppState;

const PUSH_SCOPE: &str = "ethpay.server.canpushstanding";

async fn service() -> Option<Arc<PgDataService>> {
    let database_url = std::env::var("DATABASE_URL").ok()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("DATABASE_URL is set but the database is unreachable");
    Some(Arc::new(PgDataService::new(pool)))
}

fn app(pg: &Arc<PgDataService>) -> Router {
    let auth_service = Arc::new(AuthService::with_config(
        Arc::clone(pg),
        AuthConfig::default(),
    ));
    let state = PgAppState::new(
        Arc::clone(pg),
        auth_service,
        None::<Arc<RedisEVMMonitor>>,
        Arc::new(NoOpRateProvider),
        Arc::new(server::services::email::NoopEmailSender),
    );
    server::api::router(state, false, None, None, None)
}

/// A user with the given role and a key with the given stored scope
/// (`None` = an unscoped key). Returns the raw key.
async fn seed_key(pg: &PgDataService, role: &str, scope: Option<&[&str]>) -> String {
    let user_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, kdf_params, encrypted_symmetric_key, \
         recovery_verification_hash, kdf_salt_identifier, role) \
         VALUES ($1, \
             '{\"algorithm\":\"argon2id\",\"memory_kb\":65536,\"iterations\":3,\"parallelism\":4,\"salt\":\"\"}'::jsonb, \
             '{\"ciphertext\":\"\",\"iv\":\"\",\"mac\":\"\"}'::jsonb, \
             'h', 'passkey:' || $1::text, $2)",
    )
    .bind(user_id)
    .bind(role)
    .execute(pg.pool())
    .await
    .expect("seed user");

    let key = format!("ak_standing_{}", Uuid::new_v4());
    let scope: Option<Vec<String>> = scope.map(|s| s.iter().map(|e| e.to_string()).collect());
    sqlx::query(
        "INSERT INTO api_keys (id, user_id, name, key_hash, key_prefix, permissions) \
         VALUES ($1, $2, 'standing test key', $3, 'ak_test', $4)",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(hex::encode(Sha256::digest(key.as_bytes())))
    .bind(scope)
    .execute(pg.pool())
    .await
    .expect("seed api key");
    key
}

async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    key: Option<&str>,
    body: Vec<u8>,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(key) = key {
        req = req.header("authorization", format!("Bearer {key}"));
    }
    let resp = app
        .clone()
        .oneshot(req.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn push_body(account: Uuid, version: i64, good: bool, plan: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "account_id": account,
        "version": version,
        "in_good_standing": good,
        "paid_through": null,
        "plan_name": plan,
        "checkout_url": "https://pay.example/checkout",
    }))
    .unwrap()
}

async fn push(app: &Router, key: Option<&str>, body: Vec<u8>) -> (StatusCode, Value) {
    send(app, "POST", "/plugins/entitlements", key, body).await
}

async fn held(pg: &PgDataService, account: Uuid) -> Option<(i64, bool, String)> {
    pg.get_account_standing(account).await.unwrap().map(|h| {
        (
            h.standing.version,
            h.standing.in_good_standing,
            h.standing.plan_name,
        )
    })
}

#[tokio::test]
#[ignore]
async fn version_six_then_version_five_leaves_version_six_held() {
    let Some(pg) = service().await else { return };
    let app = app(&pg);
    let key = seed_key(&pg, "server_admin", Some(&[PUSH_SCOPE])).await;
    let account = Uuid::new_v4();

    let (status, body) = push(&app, Some(&key), push_body(account, 6, true, "plan-six")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"applied": true, "version": 6}));

    let (status, body) = push(&app, Some(&key), push_body(account, 5, false, "plan-five")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an obsolete push is delivered, not failed"
    );
    assert_eq!(body, json!({"applied": false, "version": 6}));

    assert_eq!(
        held(&pg, account).await,
        Some((6, true, "plan-six".to_string()))
    );
}

#[tokio::test]
#[ignore]
async fn version_six_sent_twice_changes_nothing_and_succeeds() {
    let Some(pg) = service().await else { return };
    let app = app(&pg);
    let key = seed_key(&pg, "server_admin", Some(&[PUSH_SCOPE])).await;
    let account = Uuid::new_v4();

    let (first, _) = push(&app, Some(&key), push_body(account, 6, true, "plan-six")).await;
    let (status, body) = push(&app, Some(&key), push_body(account, 6, true, "plan-six")).await;
    assert_eq!((first, status), (StatusCode::OK, StatusCode::OK));
    assert_eq!(body, json!({"applied": false, "version": 6}));

    // A repeat carrying different content is a sender fault: still success,
    // and the held value is kept.
    let (status, _) = push(&app, Some(&key), push_body(account, 6, false, "other")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        held(&pg, account).await,
        Some((6, true, "plan-six".to_string()))
    );
}

#[tokio::test]
#[ignore]
async fn an_unauthenticated_push_is_refused_and_changes_nothing() {
    let Some(pg) = service().await else { return };
    let app = app(&pg);
    let account = Uuid::new_v4();

    let (status, _) = push(&app, None, push_body(account, 1, true, "p")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = push(
        &app,
        Some("ak_not_a_real_key"),
        push_body(account, 1, true, "p"),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    assert_eq!(held(&pg, account).await, None);
}

#[tokio::test]
#[ignore]
async fn a_key_without_the_push_scope_is_refused_whatever_else_it_may_do() {
    let Some(pg) = service().await else { return };
    let app = app(&pg);
    let account = Uuid::new_v4();

    let keys = [
        (
            "unrestricted admin key",
            seed_key(&pg, "server_admin", None).await,
        ),
        (
            "admin key scoped to unrestricted",
            seed_key(&pg, "server_admin", Some(&["unrestricted"])).await,
        ),
        (
            "admin key scoped to the merchant listing",
            seed_key(&pg, "server_admin", Some(&["ethpay.server.canviewusers"])).await,
        ),
        (
            "admin key scoped to a store permission",
            seed_key(
                &pg,
                "server_admin",
                Some(&["ethpay.store.cancreateinvoice"]),
            )
            .await,
        ),
        ("ordinary user's key", seed_key(&pg, "user", None).await),
        (
            "push scope on a non-admin owner's key",
            seed_key(&pg, "user", Some(&[PUSH_SCOPE])).await,
        ),
    ];
    for (what, key) in &keys {
        let (status, _) = push(&app, Some(key), push_body(account, 1, true, "p")).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{what}");
    }
    assert_eq!(
        held(&pg, account).await,
        None,
        "a refused push must store nothing"
    );
}

#[tokio::test]
#[ignore]
async fn the_push_key_is_good_for_nothing_but_the_push() {
    let Some(pg) = service().await else { return };
    let app = app(&pg);
    let key = seed_key(&pg, "server_admin", Some(&[PUSH_SCOPE])).await;

    for uri in [
        "/admin/stores",
        "/admin/users",
        "/admin/settings",
        "/stores",
        "/users/api-keys",
    ] {
        let (status, _) = send(&app, "GET", uri, Some(&key), Vec::new()).await;
        assert!(
            status == StatusCode::FORBIDDEN || status == StatusCode::UNAUTHORIZED,
            "GET {uri} gave {status}"
        );
    }
}

#[tokio::test]
#[ignore]
async fn malformed_and_oversized_pushes_are_refused_and_store_nothing() {
    let Some(pg) = service().await else { return };
    let app = app(&pg);
    let key = seed_key(&pg, "server_admin", Some(&[PUSH_SCOPE])).await;
    let account = Uuid::new_v4();

    let bad = [
        push_body(account, 0, true, "p"),
        push_body(account, -1, true, "p"),
        push_body(account, 1, true, ""),
        push_body(account, 1, true, &"x".repeat(201)),
        serde_json::to_vec(&json!({"account_id": "not-a-uuid", "version": 1,
            "in_good_standing": true, "plan_name": "p"}))
        .unwrap(),
        serde_json::to_vec(&json!({"account_id": account, "version": 1,
            "in_good_standing": true, "plan_name": "p",
            "checkout_url": "javascript:alert(1)"}))
        .unwrap(),
        b"not json".to_vec(),
    ];
    for body in bad {
        let (status, _) = push(&app, Some(&key), body.clone()).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{}",
            String::from_utf8_lossy(&body)
        );
    }

    let (status, _) = push(
        &app,
        Some(&key),
        push_body(account, 1, true, &"x".repeat(8192)),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);

    assert_eq!(held(&pg, account).await, None);
}

#[tokio::test]
#[ignore]
async fn authentication_is_checked_before_the_body() {
    let Some(pg) = service().await else { return };
    let app = app(&pg);

    // An invalid body from a caller with no credential must not reveal that
    // it was invalid.
    let (status, _) = push(&app, None, b"not json".to_vec()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
