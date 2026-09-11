//! Invoice creation is one unit of work, against a real database.
//!
//! The bug: the invoice, its payment options and their watched addresses were
//! three independent commits, so a failure partway left an invoice that could
//! not be paid. These assert the whole set lands together or not at all - which
//! only the real foreign keys and a real transaction can show.

use sqlx::PgPool;
use uuid::Uuid;

use crate::invoice_creation::InvoiceCreationWriter;
use crate::postgres::PgDataService;
use types::{
    ChainId, InvoiceData, InvoiceId, InvoiceStatus, PaymentMethodId, PaymentOptionData,
    PaymentOptionId, StoreId,
};

async fn service() -> Option<PgDataService> {
    let database_url = std::env::var("DATABASE_URL").ok()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .ok()?;
    Some(PgDataService::new(pool))
}

async fn seed_store(pool: &PgPool) -> (Uuid, Uuid) {
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

    let store_id = Uuid::new_v4();
    sqlx::query("INSERT INTO stores (id, name, owner_id) VALUES ($1, $2, $3)")
        .bind(store_id)
        .bind(format!("store-{store_id}"))
        .bind(user_id)
        .execute(pool)
        .await
        .expect("seed store");
    (user_id, store_id)
}

fn an_invoice(store_id: Uuid) -> InvoiceData {
    InvoiceData {
        id: InvoiceId(format!("inv-{}", Uuid::new_v4())),
        store_id: StoreId(store_id),
        currency: "USD".to_string(),
        status: InvoiceStatus::Pending,
        amount: "10".to_string(),
        amount_received: "0".to_string(),
        created_at: chrono::Utc::now(),
        expires_at: chrono::Utc::now() + chrono::Duration::hours(3),
        metadata: None,
        customer_email: None,
        extra: None,
    }
}

fn an_option(invoice: &InvoiceData, symbol: &str, token: Option<&str>) -> PaymentOptionData {
    let chain_id = ChainId::evm(11155111);
    PaymentOptionData {
        id: PaymentOptionId(Uuid::new_v4()),
        invoice_id: invoice.id.clone(),
        payment_method_id: PaymentMethodId::new(symbol, &chain_id),
        chain_id,
        asset_symbol: symbol.to_string(),
        token_address: token.map(str::to_string),
        decimals: 18,
        payment_address: format!("0x{:040x}", Uuid::new_v4().as_u128()),
        wallet_id: None,
        derivation_index: Some(0),
        amount: "1".to_string(),
        rate: None,
        rate_at: None,
        is_active: true,
        created_at: chrono::Utc::now(),
    }
}

async fn counts(pool: &PgPool, invoice_id: &str) -> (i64, i64, i64) {
    let inv: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM invoices WHERE id = $1")
        .bind(invoice_id)
        .fetch_one(pool)
        .await
        .unwrap();
    let opt: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM payment_options WHERE invoice_id = $1")
        .bind(invoice_id)
        .fetch_one(pool)
        .await
        .unwrap();
    let watched: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM watched_addresses WHERE invoice_id = $1")
            .bind(invoice_id)
            .fetch_one(pool)
            .await
            .unwrap();
    (inv, opt, watched)
}

#[tokio::test]
#[ignore]
async fn the_whole_invoice_lands_in_one_go() {
    let Some(service) = service().await else {
        return;
    };
    let (_user, store) = seed_store(&service.pool).await;
    let invoice = an_invoice(store);
    // One native and one ERC-20: they take different paths through the watched
    // address write, and only the native one needs SELECT ... FOR UPDATE.
    let options = vec![
        an_option(&invoice, "ETH", None),
        an_option(
            &invoice,
            "USDC",
            Some("0x1f9840a85d5af5bf1d1762f925bdaddc4201f984"),
        ),
    ];

    service
        .create_invoice_with_options(&invoice, &options)
        .await
        .expect("create");

    let (inv, opt, watched) = counts(&service.pool, invoice.id.as_str()).await;
    assert_eq!((inv, opt, watched), (1, 2, 2));
}

#[tokio::test]
#[ignore]
async fn a_failure_partway_leaves_nothing_at_all() {
    // THE regression. Two options where the second cannot be written - a
    // duplicate id violates the primary key - so the write fails after the
    // invoice and the first option have been inserted inside the transaction.
    //
    // Before this was transactional, that state was reachable for real: a
    // derivation error on the third of three methods left an invoice with two
    // payment options, payable in some assets and not others.
    let Some(service) = service().await else {
        return;
    };
    let (_user, store) = seed_store(&service.pool).await;
    let invoice = an_invoice(store);
    let first = an_option(&invoice, "ETH", None);
    let mut clash = an_option(
        &invoice,
        "USDC",
        Some("0xdac17f958d2ee523a2206206994597c13d831ec7"),
    );
    clash.id = PaymentOptionId(first.id.0); // same primary key

    let result = service
        .create_invoice_with_options(&invoice, &[first, clash])
        .await;

    assert!(result.is_err(), "a duplicate option id must fail the write");

    let (inv, opt, watched) = counts(&service.pool, invoice.id.as_str()).await;
    assert_eq!(
        (inv, opt, watched),
        (0, 0, 0),
        "the invoice, its options and its watched addresses must all be rolled back"
    );
}

#[tokio::test]
#[ignore]
async fn watched_addresses_take_the_invoice_expiry() {
    // The per-option writer re-read the expiry from the database and fell back
    // to "24 hours from now" when it found nothing. Passing it in removes the
    // guess; this pins that the value stored is the invoice's own.
    let Some(service) = service().await else {
        return;
    };
    let (_user, store) = seed_store(&service.pool).await;
    let invoice = an_invoice(store);
    let option = an_option(&invoice, "ETH", None);

    service
        .create_invoice_with_options(&invoice, std::slice::from_ref(&option))
        .await
        .expect("create");

    let stored: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT expires_at FROM watched_addresses WHERE payment_option_id = $1")
            .bind(option.id.0)
            .fetch_one(&service.pool)
            .await
            .expect("read watched address");

    assert_eq!(
        stored.timestamp(),
        invoice.expires_at.timestamp(),
        "the watched address must expire with its invoice, not 24h from now"
    );
}
