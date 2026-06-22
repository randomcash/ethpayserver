#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::super::*;
use ::types::{StoreTokenPolicyEntry, TokenPolicyMode};

fn make_pm(chain_id: u64, token_address: Option<&str>) -> data_service::StorePaymentMethod {
    data_service::StorePaymentMethod {
        id: uuid::Uuid::new_v4(),
        store_id: uuid::Uuid::new_v4(),
        chain_id,
        token_address: token_address.map(String::from),
        asset_symbol: "TEST".to_string(),
        decimals: 18,
        xpub: "xpub_test".to_string(),
        derivation_index: 0,
        enabled: true,
        created_at: chrono::Utc::now(),
    }
}

fn make_entry(chain_id: i64, token_address: Option<&str>) -> StoreTokenPolicyEntry {
    StoreTokenPolicyEntry {
        id: uuid::Uuid::new_v4(),
        policy_id: uuid::Uuid::new_v4(),
        chain_id,
        token_address: token_address.map(String::from),
        asset_symbol: "TEST".to_string(),
    }
}

fn make_policy(
    mode: TokenPolicyMode,
    entries: Vec<StoreTokenPolicyEntry>,
) -> data_service::StoreTokenPolicyWithEntries {
    data_service::StoreTokenPolicyWithEntries {
        id: uuid::Uuid::new_v4(),
        store_id: uuid::Uuid::new_v4(),
        mode,
        entries,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

#[test]
fn test_token_policy_allowlist_filters_correctly() {
    let mut methods = vec![
        make_pm(1, None),
        make_pm(1, Some("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48")),
        make_pm(137, None),
    ];
    let policy = make_policy(TokenPolicyMode::Allowlist, vec![make_entry(1, None)]);
    apply_token_policy_filter(&mut methods, &policy);
    assert_eq!(methods.len(), 1);
    assert_eq!(methods[0].chain_id, 1);
    assert!(methods[0].token_address.is_none());
}

#[test]
fn test_token_policy_blocklist_filters_correctly() {
    let usdc = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
    let mut methods = vec![make_pm(1, None), make_pm(1, Some(usdc)), make_pm(137, None)];
    let policy = make_policy(TokenPolicyMode::Blocklist, vec![make_entry(1, Some(usdc))]);
    apply_token_policy_filter(&mut methods, &policy);
    assert_eq!(methods.len(), 2);
    assert!(methods.iter().all(|m| m.token_address.is_none()));
}

#[test]
fn test_token_policy_native_asset_handling() {
    let usdt = "0xdAC17F958D2ee523a2206206994597C13D831ec7";
    let mut methods = vec![make_pm(1, None), make_pm(1, Some(usdt))];
    let policy = make_policy(TokenPolicyMode::Allowlist, vec![make_entry(1, None)]);
    apply_token_policy_filter(&mut methods, &policy);
    assert_eq!(methods.len(), 1);
    assert!(methods[0].token_address.is_none());
}

#[test]
fn test_token_policy_allowlist_empty_entries_blocks_all() {
    let mut methods = vec![make_pm(1, None), make_pm(137, None)];
    let policy = make_policy(TokenPolicyMode::Allowlist, vec![]);
    apply_token_policy_filter(&mut methods, &policy);
    assert!(methods.is_empty());
}

#[test]
fn test_token_policy_blocklist_empty_entries_allows_all() {
    let mut methods = vec![make_pm(1, None), make_pm(137, None)];
    let policy = make_policy(TokenPolicyMode::Blocklist, vec![]);
    apply_token_policy_filter(&mut methods, &policy);
    assert_eq!(methods.len(), 2);
}
