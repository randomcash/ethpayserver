//! `ServerSettingsRepository`, against a real database.
//!
//! These exist because the read path was broken from the day the
//! `enabled_chain_ids` column became a `caip2[]` domain, and nothing caught
//! it. The column is an array of a DOMAIN over text; sqlx decodes by type
//! OID, so asking for `Vec<String>` off a `caip2[]` fails every time, whatever
//! the values are.
//!
//! It stayed invisible because no settings row ever existed on any instance:
//! `fetch_optional` returned `None`, the decode never ran, and every caller
//! fell back to `ServerSettings::default()`. The first successful save would
//! have written a row and then crash-looped the process on the next boot,
//! since the decode was an unwrapping `get` in a value read during startup.
//!
//! A unit test could not have found it - there is no `caip2` domain outside a
//! real database - which is exactly why these live here.

use types::StoreId;
use uuid::Uuid;

use crate::postgres::PgDataService;
use auth::{ServerSettings, ServerSettingsRepository};

async fn service() -> Option<PgDataService> {
    let database_url = std::env::var("DATABASE_URL").ok()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .ok()?;
    Some(PgDataService::new(pool))
}

fn settings_with(chains: Vec<types::ChainId>, store: Option<StoreId>) -> ServerSettings {
    ServerSettings {
        default_confirmations: 5,
        invoice_expiry_minutes: 45,
        rate_limit_rpm: 250,
        enabled_chain_ids: chains,
        billing_store_id: store,
    }
}

/// The regression. Write a row, read it back - which is the sequence that
/// never worked, and which no amount of reading with no row present would
/// have exercised.
#[tokio::test]
#[ignore]
async fn a_written_settings_row_can_be_read_back() {
    let Some(service) = service().await else {
        return;
    };

    let chains = vec![types::ChainId::evm(1), types::ChainId::evm(11155111)];
    let store = StoreId(Uuid::new_v4());
    service
        .upsert_server_settings(&settings_with(chains.clone(), Some(store)))
        .await
        .expect("settings should save");

    let read = service
        .get_server_settings()
        .await
        .expect("reading settings must not fail")
        .expect("the row just written must be there");

    assert_eq!(read.default_confirmations, 5);
    assert_eq!(read.invoice_expiry_minutes, 45);
    assert_eq!(read.rate_limit_rpm, 250);
    assert_eq!(
        read.enabled_chain_ids, chains,
        "the chain ids must survive the caip2[] column intact"
    );
    assert_eq!(read.billing_store_id, Some(store));
}

/// Clearing the billing store is how an instance stops selling to itself, and
/// it has to be distinguishable from never having set one.
#[tokio::test]
#[ignore]
async fn the_billing_store_can_be_set_and_cleared() {
    let Some(service) = service().await else {
        return;
    };

    let store = StoreId(Uuid::new_v4());
    service
        .upsert_server_settings(&settings_with(vec![types::ChainId::evm(1)], Some(store)))
        .await
        .expect("save with a store");
    assert_eq!(
        service
            .get_server_settings()
            .await
            .unwrap()
            .unwrap()
            .billing_store_id,
        Some(store)
    );

    service
        .upsert_server_settings(&settings_with(vec![types::ChainId::evm(1)], None))
        .await
        .expect("save without a store");
    assert_eq!(
        service
            .get_server_settings()
            .await
            .unwrap()
            .unwrap()
            .billing_store_id,
        None,
        "clearing must actually clear, not leave the previous store in place"
    );
}

/// An empty list is a real answer - a server that enables no chains from
/// settings - and must not be confused with a decode that gave up.
#[tokio::test]
#[ignore]
async fn no_enabled_chains_round_trips_as_an_empty_list() {
    let Some(service) = service().await else {
        return;
    };

    service
        .upsert_server_settings(&settings_with(Vec::new(), None))
        .await
        .expect("save with no chains");

    let read = service.get_server_settings().await.unwrap().unwrap();
    assert!(read.enabled_chain_ids.is_empty());
}
