//! Store creation is one unit of work, against a real database.
//!
//! The bug: the store row, the Owner role lookup and the membership write were
//! three independent steps, the first of which committed on its own. A failure
//! after it left a store owned by nobody — invisible in a UI that lists stores by
//! membership, and undeletable through it. Only real foreign keys and a real
//! transaction can show that the pair lands together or not at all.

use sqlx::PgPool;
use uuid::Uuid;

use crate::postgres::PgDataService;
use crate::store_creation::{StoreCreationError, StoreCreationWriter};
use auth::{Store, UserId};

async fn service() -> Option<PgDataService> {
    let database_url = std::env::var("DATABASE_URL").ok()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .ok()?;
    Some(PgDataService::new(pool))
}

async fn seed_user(pool: &PgPool) -> Uuid {
    let user_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, kdf_params, encrypted_symmetric_key, \
         recovery_verification_hash, kdf_salt_identifier) \
         VALUES ($1, '{}'::jsonb, '{}'::jsonb, 'h', 'passkey:' || $1::text)",
    )
    .bind(user_id)
    .execute(pool)
    .await
    .expect("seed user");
    user_id
}

async fn counts(pool: &PgPool, store_id: Uuid) -> (i64, i64) {
    let stores: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stores WHERE id = $1")
        .bind(store_id)
        .fetch_one(pool)
        .await
        .unwrap();
    let memberships: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM user_stores WHERE store_id = $1")
            .bind(store_id)
            .fetch_one(pool)
            .await
            .unwrap();
    (stores, memberships)
}

#[tokio::test]
#[ignore]
async fn a_store_and_its_ownership_land_together() {
    let Some(service) = service().await else {
        return;
    };
    let user = seed_user(&service.pool).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(user));

    let membership = service
        .create_store_owned_by(&store, UserId(user))
        .await
        .expect("create");

    assert_eq!(membership.user_id.0, user);
    assert_eq!(membership.store_id, store.id);

    let (stores, memberships) = counts(&service.pool, store.id.0).await;
    assert_eq!(
        (stores, memberships),
        (1, 1),
        "a created store must come with the membership that owns it"
    );
}

#[tokio::test]
#[ignore]
async fn an_absent_role_reads_as_none_which_is_what_becomes_missing_owner_role() {
    // `get_default_role_by_name("Owner")` returning None is the failure that
    // surfaced this bug: the global default roles seeded by migration
    // 20241214000001 had been deleted, every creation returned 500, and every
    // `stores` row was left behind owned by nobody.
    //
    // What this asserts is the lookup's None, which is the exact input to the
    // writer's `.ok_or(StoreCreationError::MissingOwnerRole)`.
    //
    // What it deliberately does not do is delete the seeded Owner role and call
    // the writer. Two reasons, both real: the role is global, so any `user_stores`
    // row referencing it makes the delete fail on a foreign key — which is what
    // happened when this test was first written that way — and the writer opens
    // its own transaction, so it could not observe a deletion staged in the
    // test's transaction anyway. A test that cannot see the state it sets up is
    // worse than one that says what it covers.
    let Some(service) = service().await else {
        return;
    };
    let mut conn = service.pool.acquire().await.expect("connection");

    let found = crate::postgres::store_creation::default_role_by_name(&mut conn, "Owner")
        .await
        .expect("lookup");
    assert!(
        found.is_some(),
        "the seed migration's Owner role must exist, or store creation cannot work at all"
    );

    let absent =
        crate::postgres::store_creation::default_role_by_name(&mut conn, "NoSuchDefaultRoleName")
            .await
            .expect("lookup");
    assert!(
        absent.is_none(),
        "an absent role must read as None rather than erroring — None is what the \
         writer turns into MissingOwnerRole"
    );
}

#[tokio::test]
#[ignore]
async fn a_duplicate_store_id_changes_nothing() {
    // A second creation with the same id must fail and leave the first one's rows
    // exactly as they were.
    //
    // Note what this does *not* prove: the duplicate fails on the `stores` primary
    // key, so it fails at the first statement and there is nothing to roll back.
    // The rollback itself is proved by
    // `the_store_insert_rolls_back_when_the_membership_fails` below.
    let Some(service) = service().await else {
        return;
    };
    let user = seed_user(&service.pool).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(user));

    service
        .create_store_owned_by(&store, UserId(user))
        .await
        .expect("first create");

    let again = service.create_store_owned_by(&store, UserId(user)).await;
    assert!(
        matches!(again, Err(StoreCreationError::Repository(_))),
        "creating the same store id twice must fail"
    );

    let (stores, memberships) = counts(&service.pool, store.id.0).await;
    assert_eq!(
        (stores, memberships),
        (1, 1),
        "the failed second attempt must not have added or removed anything"
    );
}

#[tokio::test]
#[ignore]
async fn the_store_insert_rolls_back_when_the_membership_fails() {
    // THE regression, and the only test here that actually proves it.
    //
    // The original code committed the store, then looked up the role, then wrote
    // the membership. When either of the last two failed the caller got a 500 and
    // the `stores` row stayed — owned by nobody, invisible in a UI that lists by
    // membership, undeletable through it. Observed for real: every creation
    // returned 500 and every row was still in `stores`.
    //
    // Driven at the statement level because the failure cannot be forced through
    // `create_store_owned_by` from outside: the membership write only fails on a
    // bad role id, and the writer sources that id from its own lookup. So this
    // runs the same two statements the writer runs, in one transaction, with the
    // second one guaranteed to fail.
    let Some(service) = service().await else {
        return;
    };
    let user = seed_user(&service.pool).await;
    let store = Store::new(format!("store-{}", Uuid::new_v4()), UserId(user));

    let mut tx = service.pool.begin().await.expect("begin");
    crate::postgres::store_creation::insert_store(&mut tx, &store)
        .await
        .expect("the store insert itself must succeed");

    // Inside the transaction the store is visible, which is exactly the window
    // the old code committed in.
    let mid: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stores WHERE id = $1")
        .bind(store.id.0)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(mid, 1, "the store should exist inside the transaction");

    // A role id that does not exist violates user_stores_store_role_id_fkey.
    let doomed = auth::UserStore::new(UserId(user), store.id, auth::StoreRoleId(Uuid::new_v4()));
    let failed = crate::postgres::store_creation::insert_user_store(&mut tx, &doomed).await;
    assert!(
        failed.is_err(),
        "an unknown role id must fail the membership write"
    );

    // Dropping without commit is the rollback every `?` in the writer relies on.
    drop(tx);

    let (stores, memberships) = counts(&service.pool, store.id.0).await;
    assert_eq!(
        (stores, memberships),
        (0, 0),
        "a failed membership write must take the store row with it — this is the \
         orphaned, unowned store the bug left behind"
    );
}
