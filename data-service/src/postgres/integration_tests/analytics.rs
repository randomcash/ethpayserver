//! Payment analytics integration tests (RCS-225).
//!
//! These pin the rules the in-memory double in `test_utils` also implements.
//! A double that disagrees with Postgres about scoping or the window is the
//! RCS-203 failure mode, so both sides are asserted against the same
//! expectations.

use chrono::{Duration, Utc};
use types::{InvoiceWriter, PaymentOptionWriter, PaymentWriter};

use crate::analytics::{PaymentAnalyticsReader, PaymentVolumeQuery};

use super::super::PgDataService;
use super::{
    create_test_service, seeded_test_invoice, test_payment, test_payment_option_with_rate,
};

/// A 30-day window ending at the end of today (UTC), the endpoint's default.
fn last_30_days() -> PaymentVolumeQuery {
    let today = Utc::now().date_naive();
    PaymentVolumeQuery {
        store_ids: Vec::new(),
        since: (today - Duration::days(29))
            .and_hms_opt(0, 0, 0)
            .expect("midnight is a valid time")
            .and_utc(),
        until: (today + Duration::days(1))
            .and_hms_opt(0, 0, 0)
            .expect("midnight is a valid time")
            .and_utc(),
    }
}

/// Seed one invoice in a fresh store with a single 1 ETH payment.
async fn seed_one_eth_payment(service: &PgDataService) -> (types::StoreId, types::InvoiceId) {
    let invoice = seeded_test_invoice(service).await;
    InvoiceWriter::upsert(service, &invoice).await.unwrap();
    let payment = test_payment(&invoice.id);
    PaymentWriter::upsert(service, &payment).await.unwrap();
    (invoice.store_id, invoice.id)
}

#[tokio::test]
#[ignore]
async fn integration_analytics_empty_store_list_reads_nothing() {
    // "No stores" must filter everything out. Treating it as "all stores" is
    // how a user who belongs to no store ends up reading the whole server
    // (RCS-222, RCS-211).
    let service = create_test_service().await.expect("DATABASE_URL required");
    seed_one_eth_payment(&service).await;

    let buckets = service
        .payment_volume_by_day(&last_30_days())
        .await
        .unwrap();
    assert!(buckets.is_empty());
}

#[tokio::test]
#[ignore]
async fn integration_analytics_scopes_to_the_named_stores() {
    let service = create_test_service().await.expect("DATABASE_URL required");
    let (mine, _) = seed_one_eth_payment(&service).await;
    let (theirs, _) = seed_one_eth_payment(&service).await;
    assert_ne!(mine, theirs);

    let query = PaymentVolumeQuery {
        store_ids: vec![mine],
        ..last_30_days()
    };
    let buckets = service.payment_volume_by_day(&query).await.unwrap();

    assert_eq!(buckets.len(), 1);
    assert_eq!(buckets[0].asset_symbol, "ETH");
    assert_eq!(buckets[0].payment_count, 1);
    assert_eq!(buckets[0].day, Utc::now().date_naive());
    // No payment option on the payment, so decimals fall back to 18.
    assert_eq!(buckets[0].decimals, 18);
    assert_eq!(buckets[0].raw_amount, "1000000000000000000");
}

#[tokio::test]
#[ignore]
async fn integration_analytics_uses_the_payment_options_decimals() {
    let service = create_test_service().await.expect("DATABASE_URL required");
    let invoice = seeded_test_invoice(&service).await;
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    let option = test_payment_option_with_rate(
        &invoice.id,
        1,
        "USDC",
        Some("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string()),
        6,
        "100000000",
        Some("1.0".to_string()),
    );
    PaymentOptionWriter::create(&service, &option)
        .await
        .unwrap();

    let mut payment = test_payment(&invoice.id);
    payment.payment_option_id = Some(option.id.0);
    payment.asset_symbol = "USDC".to_string();
    payment.asset_type = types::AssetType::ERC20;
    payment.token_address = option.token_address.clone();
    payment.amount = "2500000".to_string();
    PaymentWriter::upsert(&service, &payment).await.unwrap();

    let query = PaymentVolumeQuery {
        store_ids: vec![invoice.store_id],
        ..last_30_days()
    };
    let buckets = service.payment_volume_by_day(&query).await.unwrap();

    assert_eq!(buckets.len(), 1);
    assert_eq!(buckets[0].decimals, 6, "must come from the payment option");
    assert_eq!(buckets[0].raw_amount, "2500000");
}

#[tokio::test]
#[ignore]
async fn integration_analytics_excludes_reorged_payments() {
    // A reorged payment was rolled back by the chain; charting it would show
    // a merchant money that never arrived.
    let service = create_test_service().await.expect("DATABASE_URL required");
    let invoice = seeded_test_invoice(&service).await;
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    let mut payment = test_payment(&invoice.id);
    payment.block_number = Some(100);
    PaymentWriter::upsert(&service, &payment).await.unwrap();
    PaymentWriter::mark_reorged(&service, &invoice.id, 1, 100)
        .await
        .unwrap();

    let query = PaymentVolumeQuery {
        store_ids: vec![invoice.store_id],
        ..last_30_days()
    };
    let buckets = service.payment_volume_by_day(&query).await.unwrap();
    assert!(buckets.is_empty());
}

#[tokio::test]
#[ignore]
async fn integration_analytics_window_is_bounded() {
    let service = create_test_service().await.expect("DATABASE_URL required");
    let invoice = seeded_test_invoice(&service).await;
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    let mut old_payment = test_payment(&invoice.id);
    old_payment.detected_at = Utc::now() - Duration::days(120);
    PaymentWriter::upsert(&service, &old_payment).await.unwrap();

    let query = PaymentVolumeQuery {
        store_ids: vec![invoice.store_id],
        ..last_30_days()
    };
    let buckets = service.payment_volume_by_day(&query).await.unwrap();
    assert!(
        buckets.is_empty(),
        "history older than the window must not be aggregated"
    );
}

#[tokio::test]
#[ignore]
async fn integration_analytics_groups_by_day_and_asset() {
    let service = create_test_service().await.expect("DATABASE_URL required");
    let invoice = seeded_test_invoice(&service).await;
    InvoiceWriter::upsert(&service, &invoice).await.unwrap();

    let today = Utc::now().date_naive();
    let yesterday = today - Duration::days(1);

    for _ in 0..2 {
        let mut payment = test_payment(&invoice.id);
        payment.detected_at = Utc::now();
        PaymentWriter::upsert(&service, &payment).await.unwrap();
    }
    let mut earlier = test_payment(&invoice.id);
    earlier.detected_at = Utc::now() - Duration::days(1);
    PaymentWriter::upsert(&service, &earlier).await.unwrap();

    let query = PaymentVolumeQuery {
        store_ids: vec![invoice.store_id],
        ..last_30_days()
    };
    let buckets = service.payment_volume_by_day(&query).await.unwrap();

    assert_eq!(buckets.len(), 2, "one bucket per (day, asset, decimals)");
    assert_eq!(buckets[0].day, yesterday, "ordered by day ascending");
    assert_eq!(buckets[0].payment_count, 1);
    assert_eq!(buckets[1].day, today);
    assert_eq!(buckets[1].payment_count, 2);
    assert_eq!(buckets[1].raw_amount, "2000000000000000000");
}
