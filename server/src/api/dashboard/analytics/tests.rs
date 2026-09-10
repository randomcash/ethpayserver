#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use types::ChainId;

fn day(iso: &str) -> NaiveDate {
    NaiveDate::parse_from_str(iso, "%Y-%m-%d").unwrap()
}

fn bucket(date: &str, symbol: &str, decimals: u8, raw: &str, count: i64) -> PaymentVolumeBucket {
    PaymentVolumeBucket {
        day: day(date),
        asset_symbol: symbol.to_string(),
        decimals,
        raw_amount: raw.to_string(),
        payment_count: count,
    }
}

// =========================================================================
// Window validation
// =========================================================================

#[test]
fn days_defaults_to_thirty() {
    assert_eq!(validate_days(None).unwrap(), DEFAULT_WINDOW_DAYS);
}

#[test]
fn days_accepts_the_windows_the_chart_offers() {
    for d in [7, 30, 90] {
        assert_eq!(validate_days(Some(d)).unwrap(), d);
    }
}

#[test]
fn days_rejects_zero_and_absurd_windows() {
    // Never aggregate unbounded history, and never quietly substitute a
    // different window than the caller asked for.
    assert_eq!(validate_days(Some(0)), Err(StatusCode::BAD_REQUEST));
    assert_eq!(
        validate_days(Some(MAX_WINDOW_DAYS + 1)),
        Err(StatusCode::BAD_REQUEST)
    );
    assert_eq!(validate_days(Some(u32::MAX)), Err(StatusCode::BAD_REQUEST));
}

// =========================================================================
// Empty accounts
// =========================================================================

#[test]
fn empty_account_gets_a_valid_empty_window() {
    let out = build_analytics(&[], day("2026-08-09"), 30);
    assert!(out.assets.is_empty(), "no payments must mean no series");
    assert_eq!(out.total_payments, 0);
    assert_eq!(out.days, 30);
    assert_eq!(out.start_date, day("2026-08-09"));
    assert_eq!(out.end_date, day("2026-09-07"));
}

// =========================================================================
// Scaling and grouping
// =========================================================================

#[test]
fn amounts_are_scaled_by_their_own_decimals() {
    let buckets = [
        bucket("2026-09-01", "ETH", 18, "1500000000000000000", 1),
        bucket("2026-09-01", "USDC", 6, "2500000", 1),
    ];
    let out = build_analytics(&buckets, day("2026-09-01"), 1);

    let eth = out.assets.iter().find(|a| a.asset_symbol == "ETH").unwrap();
    let usdc = out
        .assets
        .iter()
        .find(|a| a.asset_symbol == "USDC")
        .unwrap();
    assert_eq!(eth.total_amount, "1.5");
    assert_eq!(usdc.total_amount, "2.5");
}

#[test]
fn same_asset_with_different_decimals_is_summed_in_whole_units() {
    // The repository keeps `decimals` in the group key, so one asset can
    // arrive as two buckets. Summing the raw amounts would be nonsense; the
    // whole-unit values add up.
    let buckets = [
        bucket("2026-09-01", "USDC", 6, "1000000", 1),
        bucket("2026-09-01", "USDC", 18, "2000000000000000000", 1),
    ];
    let out = build_analytics(&buckets, day("2026-09-01"), 1);

    assert_eq!(out.assets.len(), 1);
    assert_eq!(out.assets[0].total_amount, "3");
    assert_eq!(out.assets[0].payment_count, 2);
    assert_eq!(out.assets[0].daily[0].amount, "3");
}

#[test]
fn missing_days_are_zero_filled_not_omitted() {
    let buckets = [bucket("2026-09-03", "ETH", 18, "1000000000000000000", 1)];
    let out = build_analytics(&buckets, day("2026-09-01"), 5);

    let daily = &out.assets[0].daily;
    assert_eq!(daily.len(), 5, "one point per day of the window");
    assert_eq!(daily[0].date, day("2026-09-01"));
    assert_eq!(daily[4].date, day("2026-09-05"));
    assert_eq!(daily[0].amount, "0");
    assert_eq!(daily[0].payment_count, 0);
    assert_eq!(daily[2].amount, "1");
    assert_eq!(daily[2].payment_count, 1);
}

#[test]
fn amount_beyond_decimal_range_is_dropped_rather_than_failing_the_panel() {
    // 40 digits of wei is not a payment; it must not take the chart down.
    let buckets = [
        bucket("2026-09-01", "GHOST", 18, &"9".repeat(40), 3),
        bucket("2026-09-01", "ETH", 18, "1000000000000000000", 1),
    ];
    let out = build_analytics(&buckets, day("2026-09-01"), 1);

    assert_eq!(out.assets.len(), 1);
    assert_eq!(out.assets[0].asset_symbol, "ETH");
    assert_eq!(
        out.total_payments, 1,
        "a dropped bucket must not inflate the count it no longer contributes to"
    );
}

// =========================================================================
// Methods breakdown
// =========================================================================

#[test]
fn share_is_a_fraction_of_payment_count_not_of_value() {
    // 1 ETH and 3 USDC: by value ETH dominates, by count USDC does. The
    // breakdown reports the countable one.
    let buckets = [
        bucket("2026-09-01", "ETH", 18, "1000000000000000000", 1),
        bucket("2026-09-01", "USDC", 6, "3000000", 3),
    ];
    let out = build_analytics(&buckets, day("2026-09-01"), 1);

    assert_eq!(out.assets[0].asset_symbol, "USDC", "busiest asset first");
    assert_eq!(out.assets[0].share_percent, 75.0);
    assert_eq!(out.assets[1].asset_symbol, "ETH");
    assert_eq!(out.assets[1].share_percent, 25.0);
    assert_eq!(out.total_payments, 4);
}

#[test]
fn assets_with_equal_counts_are_ordered_deterministically() {
    let buckets = [
        bucket("2026-09-01", "USDT", 6, "1000000", 1),
        bucket("2026-09-01", "DAI", 18, "1000000000000000000", 1),
    ];
    let out = build_analytics(&buckets, day("2026-09-01"), 1);
    let symbols: Vec<&str> = out.assets.iter().map(|a| a.asset_symbol.as_str()).collect();
    assert_eq!(symbols, ["DAI", "USDT"]);
}

// =========================================================================
// Serialization — the client deserializes these field names
// =========================================================================

#[test]
fn response_field_names_match_the_client() {
    let out = build_analytics(
        &[bucket("2026-09-01", "ETH", 18, "1000000000000000000", 1)],
        day("2026-09-01"),
        1,
    );
    let json = serde_json::to_value(&out).unwrap();

    assert_eq!(json["days"], 1);
    assert_eq!(json["start_date"], "2026-09-01");
    assert_eq!(json["end_date"], "2026-09-01");
    assert_eq!(json["total_payments"], 1);

    let asset = &json["assets"][0];
    assert_eq!(asset["asset_symbol"], "ETH");
    assert_eq!(asset["total_amount"], "1");
    assert_eq!(asset["payment_count"], 1);
    assert_eq!(asset["share_percent"], 100.0);
    assert_eq!(asset["daily"][0]["date"], "2026-09-01");
    assert_eq!(asset["daily"][0]["amount"], "1");
    assert_eq!(asset["daily"][0]["payment_count"], 1);
}

// =========================================================================
// Store scoping, against the in-memory repository double
// =========================================================================

mod repository {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use chrono::Duration;
    use data_service::InMemoryDataService;
    use types::{
        InvoiceData, InvoiceId, InvoiceStatus, InvoiceWriter, PaymentData, PaymentWriter, StoreId,
    };

    fn invoice(store_id: StoreId) -> InvoiceData {
        InvoiceData {
            id: InvoiceId::new(),
            store_id,
            currency: "USD".to_string(),
            status: InvoiceStatus::Pending,
            amount: "100".to_string(),
            amount_received: "0".to_string(),
            created_at: Utc::now(),
            expires_at: Utc::now() + Duration::hours(1),
            metadata: None,
            customer_email: None,
            extra: None,
        }
    }

    fn payment(invoice_id: &InvoiceId, detected_at: DateTime<Utc>) -> PaymentData {
        PaymentData {
            id: uuid::Uuid::new_v4(),
            invoice_id: invoice_id.clone(),
            payment_option_id: None,
            chain_id: ChainId::parse("eip155:1").unwrap(),
            asset_type: types::AssetType::Native,
            amount: "1000000000000000000".to_string(),
            asset_symbol: "ETH".to_string(),
            token_address: None,
            tx_hash: format!("0x{:064x}", uuid::Uuid::new_v4().as_u128()),
            block_number: Some(1),
            detected_at,
            confirmed_at: None,
            from_address: None,
            reorged: false,
            extra: None,
            credited_amount: None,
            rate_used: None,
            rate_applied_at: None,
        }
    }

    async fn seed(ds: &InMemoryDataService, store_id: StoreId, detected_at: DateTime<Utc>) {
        let inv = invoice(store_id);
        InvoiceWriter::upsert(ds, &inv).await.unwrap();
        PaymentWriter::upsert(ds, &payment(&inv.id, detected_at))
            .await
            .unwrap();
    }

    fn window(store_ids: Vec<StoreId>, now: DateTime<Utc>) -> PaymentVolumeQuery {
        PaymentVolumeQuery {
            store_ids,
            since: start_of_utc_day(now.date_naive() - Duration::days(29)),
            until: start_of_utc_day(now.date_naive() + Duration::days(1)),
        }
    }

    #[tokio::test]
    async fn no_stores_reads_nothing() {
        // An empty store list is "no stores", never "every store" — that
        // collapse is how a caller reads the whole server.
        let ds = InMemoryDataService::new();
        let now = Utc::now();
        seed(&ds, StoreId::new(), now).await;

        let out = ds
            .payment_volume_by_day(&window(vec![], now))
            .await
            .unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn other_stores_payments_are_not_visible() {
        let ds = InMemoryDataService::new();
        let now = Utc::now();
        let mine = StoreId::new();
        let theirs = StoreId::new();
        seed(&ds, mine, now).await;
        seed(&ds, theirs, now).await;
        seed(&ds, theirs, now).await;

        let out = ds
            .payment_volume_by_day(&window(vec![mine], now))
            .await
            .unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].payment_count, 1);
        assert_eq!(out[0].decimals, 18, "no payment option falls back to 18");
    }

    #[tokio::test]
    async fn payments_outside_the_window_are_excluded() {
        let ds = InMemoryDataService::new();
        let now = Utc::now();
        let store = StoreId::new();
        seed(&ds, store, now - Duration::days(120)).await;

        let out = ds
            .payment_volume_by_day(&window(vec![store], now))
            .await
            .unwrap();
        assert!(out.is_empty(), "a bounded window must stay bounded");
    }

    #[tokio::test]
    async fn reorged_payments_are_excluded() {
        // A reorged payment was rolled back by the chain. Charting it shows a
        // merchant money that never arrived.
        let ds = InMemoryDataService::new();
        let now = Utc::now();
        let store = StoreId::new();
        let inv = invoice(store);
        InvoiceWriter::upsert(&ds, &inv).await.unwrap();
        let mut reorged = payment(&inv.id, now);
        reorged.reorged = true;
        PaymentWriter::upsert(&ds, &reorged).await.unwrap();

        let out = ds
            .payment_volume_by_day(&window(vec![store], now))
            .await
            .unwrap();
        assert!(out.is_empty());
    }
}
