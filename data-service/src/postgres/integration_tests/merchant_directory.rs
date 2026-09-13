//! `MerchantDirectoryReader`, against a real database.
//!
//! What matters here is that the plugin-facing list is server-wide - not
//! scoped to a session or a store, since a plugin has neither - and that it
//! reads back exactly the identifiers seeded, not a projection that silently
//! drops or renames a column.

use sqlx::PgPool;
use uuid::Uuid;

use crate::merchant_directory::MerchantDirectoryReader;
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

async fn seed_user(pool: &PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, kdf_params, encrypted_symmetric_key, \
         recovery_verification_hash, kdf_salt_identifier) \
         VALUES ($1, '{}'::jsonb, '{}'::jsonb, 'h', 'passkey:' || $1::text)",
    )
    .bind(id)
    .execute(pool)
    .await
    .expect("seed user");
    id
}

async fn seed_store(pool: &PgPool, owner: Uuid, name: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO stores (id, name, owner_id) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(name)
        .bind(owner)
        .execute(pool)
        .await
        .expect("seed store");
    id
}

async fn cleanup(pool: &PgPool, user: Uuid) {
    use sqlx::Executor;
    let _ = pool
        .execute(sqlx::query("DELETE FROM users WHERE id = $1").bind(user))
        .await;
}

#[tokio::test]
#[ignore]
async fn lists_a_seeded_account_and_its_store() {
    let Some(service) = service().await else {
        return;
    };
    let user = seed_user(&service.pool).await;
    let store = seed_store(&service.pool, user, "merchant-directory-test").await;

    let accounts = service
        .list_accounts(0, 10_000)
        .await
        .expect("list accounts");
    assert!(
        accounts.iter().any(|a| a.id.0 == user),
        "the seeded account must appear in the directory"
    );

    let stores = service.list_stores(0, 10_000).await.expect("list stores");
    let found = stores
        .iter()
        .find(|s| s.id.0 == store)
        .expect("the seeded store must appear in the directory");
    assert_eq!(found.owner_id.0, user);
    assert_eq!(found.name, "merchant-directory-test");
    assert!(!found.archived);

    cleanup(&service.pool, user).await;
}

#[tokio::test]
#[ignore]
async fn limit_bounds_the_page() {
    let Some(service) = service().await else {
        return;
    };
    let user = seed_user(&service.pool).await;
    seed_store(&service.pool, user, "merchant-directory-limit-a").await;
    seed_store(&service.pool, user, "merchant-directory-limit-b").await;

    let page = service.list_stores(0, 1).await.expect("list stores");
    assert_eq!(page.len(), 1, "limit must bound the page size");

    cleanup(&service.pool, user).await;
}
