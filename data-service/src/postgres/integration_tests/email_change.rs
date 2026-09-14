//! Email-change verification against a real database (RCS-263).
//!
//! The property this module exists to pin: `kdf_salt_identifier` is the
//! identifier the recovery KDF is salted with at registration, and it must
//! never move - not even implicitly, as a side effect of confirming an email
//! change. `auth::UserRepository::update_user` already refuses a *changed*
//! value; what these tests cover is the code path this feature actually
//! walks - fetch the user, change only `email`, write it back - and confirm
//! the salt identifier (and the recovery hash it protects) come back
//! untouched. That is also what "recovery still works after an email change"
//! means: recovery verifies against those two fields, never against whatever
//! address was used to look the account up.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::email_change::EmailChangeWriter;
use crate::postgres::PgDataService;

async fn service() -> Option<PgDataService> {
    let database_url = std::env::var("DATABASE_URL").ok()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .ok()?;
    Some(PgDataService::new(pool))
}

async fn seed_user_with_email(pool: &PgPool, email: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, email, kdf_params, encrypted_symmetric_key, \
         recovery_verification_hash, kdf_salt_identifier) \
         VALUES ($1, $2, \
         '{\"algorithm\":\"argon2id\",\"memory_kb\":65536,\"iterations\":3,\"parallelism\":4,\"salt\":\"AAAAAAAAAAAAAAAAAAAAAA==\"}'::jsonb, \
         '{\"ciphertext\":\"AAAA\",\"iv\":\"AAAA\",\"mac\":\"AAAA\"}'::jsonb, \
         'original-hash', 'email:' || $2)",
    )
    .bind(id)
    .bind(email)
    .execute(pool)
    .await
    .expect("seed user");
    id
}

async fn cleanup(pool: &PgPool, user: Uuid) {
    let _ = sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user)
        .execute(pool)
        .await;
}

/// Applies a confirmed change exactly the way `confirm_email_change`
/// (server/src/api/users.rs) does: fetch the full user, mutate only `email`,
/// write it back.
async fn apply_confirmed_change(service: &PgDataService, user_id: auth::UserId, new_email: &str) {
    let mut user = auth::UserRepository::get_user(service, user_id)
        .await
        .expect("get user")
        .expect("user exists");
    user.email = Some(new_email.to_string());
    auth::UserRepository::update_user(service, &user)
        .await
        .expect("update user");
}

#[tokio::test]
#[ignore]
async fn confirming_a_change_updates_email_and_leaves_the_recovery_salt_and_hash_untouched() {
    let Some(service) = service().await else {
        return;
    };
    let user_id = seed_user_with_email(&service.pool, "original@example.com").await;
    let before = auth::UserRepository::get_user(&service, auth::UserId(user_id))
        .await
        .expect("get user")
        .expect("user exists");

    apply_confirmed_change(&service, auth::UserId(user_id), "changed@example.com").await;

    let after = auth::UserRepository::get_user(&service, auth::UserId(user_id))
        .await
        .expect("get user")
        .expect("user exists");

    assert_eq!(after.email.as_deref(), Some("changed@example.com"));
    assert_eq!(
        after.kdf_salt_identifier, before.kdf_salt_identifier,
        "the recovery KDF's salt identifier must never move, or every \
         recovery phrase issued before this change stops deriving the right key"
    );
    assert_eq!(
        after.recovery_verification_hash, before.recovery_verification_hash,
        "recovery still needs to accept the same phrase after the email changes"
    );

    // The other half of "recovery still works": the account is now found by
    // its new address, even though the salt it verifies against did not move.
    let by_new_email = auth::UserRepository::get_user_by_email(&service, "changed@example.com")
        .await
        .expect("lookup by new email")
        .expect("found by new email");
    assert_eq!(by_new_email.id, auth::UserId(user_id));

    let by_old_email = auth::UserRepository::get_user_by_email(&service, "original@example.com")
        .await
        .expect("lookup by old email");
    assert!(
        by_old_email.is_none(),
        "the old address must stop resolving once the change lands"
    );

    cleanup(&service.pool, user_id).await;
}

#[tokio::test]
#[ignore]
async fn an_expired_token_is_rejected() {
    let Some(service) = service().await else {
        return;
    };
    let user_id = seed_user_with_email(&service.pool, "expiring@example.com").await;

    let request = service
        .create_email_change_request(
            auth::UserId(user_id),
            "new@example.com",
            Utc::now() - chrono::Duration::minutes(1),
        )
        .await
        .expect("create request");

    let consumed = service
        .consume_email_change_request(request.token)
        .await
        .expect("consume attempt");
    assert!(
        consumed.is_none(),
        "an expired token must not be redeemable"
    );

    cleanup(&service.pool, user_id).await;
}

#[tokio::test]
#[ignore]
async fn a_token_cannot_be_redeemed_twice() {
    let Some(service) = service().await else {
        return;
    };
    let user_id = seed_user_with_email(&service.pool, "reused@example.com").await;

    let request = service
        .create_email_change_request(
            auth::UserId(user_id),
            "new@example.com",
            Utc::now() + chrono::Duration::minutes(30),
        )
        .await
        .expect("create request");

    let first = service
        .consume_email_change_request(request.token)
        .await
        .expect("first consume");
    assert!(first.is_some(), "the first use should succeed");

    let second = service
        .consume_email_change_request(request.token)
        .await
        .expect("second consume attempt");
    assert!(second.is_none(), "a token must not be redeemable twice");

    cleanup(&service.pool, user_id).await;
}

/// The property `a_new_request_supersedes_the_old_one` cannot see: that test
/// `await`s the first call to completion before starting the second, so it
/// only ever exercises the sequential case. Two callers racing for real -
/// e.g. a double-submitted click, or a resubmission before the first
/// request's transaction has committed - must still end up with exactly one
/// live, unconsumed row for the user; the unique partial index plus
/// `INSERT ... ON CONFLICT ... DO UPDATE` in `create_email_change_request`
/// exist specifically to make that true regardless of interleaving.
#[tokio::test]
#[ignore]
async fn concurrent_requests_never_leave_two_live_tokens() {
    let Some(service) = service().await else {
        return;
    };
    let service = std::sync::Arc::new(service);
    let user_id = seed_user_with_email(&service.pool, "racer@example.com").await;

    let a = {
        let service = service.clone();
        tokio::spawn(async move {
            service
                .create_email_change_request(
                    auth::UserId(user_id),
                    "a@example.com",
                    Utc::now() + chrono::Duration::minutes(30),
                )
                .await
        })
    };
    let b = {
        let service = service.clone();
        tokio::spawn(async move {
            service
                .create_email_change_request(
                    auth::UserId(user_id),
                    "b@example.com",
                    Utc::now() + chrono::Duration::minutes(30),
                )
                .await
        })
    };

    let (a, b) = (
        a.await.expect("task a").expect("request a"),
        b.await.expect("task b").expect("request b"),
    );

    let live: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM email_change_requests \
         WHERE user_id = $1 AND consumed_at IS NULL",
    )
    .bind(user_id)
    .fetch_one(&service.pool)
    .await
    .expect("count live requests");
    assert_eq!(
        live, 1,
        "two concurrent requests for the same user must never leave two \
         live tokens, whichever won"
    );

    // Exactly one of the two returned tokens - whichever request landed
    // last - can still be redeemed; the other must already be stale, the
    // same guarantee `a_new_request_supersedes_the_old_one` checks for the
    // sequential case.
    let a_consumable = service
        .consume_email_change_request(a.token)
        .await
        .expect("consume attempt a")
        .is_some();
    let b_consumable = service
        .consume_email_change_request(b.token)
        .await
        .expect("consume attempt b")
        .is_some();
    assert_ne!(
        a_consumable, b_consumable,
        "exactly one of the two racing requests should still be redeemable, not both and not neither"
    );

    cleanup(&service.pool, user_id).await;
}

#[tokio::test]
#[ignore]
async fn a_new_request_supersedes_the_old_one() {
    let Some(service) = service().await else {
        return;
    };
    let user_id = seed_user_with_email(&service.pool, "super@example.com").await;

    let first = service
        .create_email_change_request(
            auth::UserId(user_id),
            "wrong@example.com",
            Utc::now() + chrono::Duration::minutes(30),
        )
        .await
        .expect("create first request");

    let _second = service
        .create_email_change_request(
            auth::UserId(user_id),
            "right@example.com",
            Utc::now() + chrono::Duration::minutes(30),
        )
        .await
        .expect("create second request");

    let stale = service
        .consume_email_change_request(first.token)
        .await
        .expect("consume attempt on superseded token");
    assert!(
        stale.is_none(),
        "a superseded token must not still be redeemable"
    );

    cleanup(&service.pool, user_id).await;
}
