//! Random key material shared by `api_keys` (the raw key itself) and
//! `wallets` (the reauth-challenge token) - split out so neither file has to
//! depend on the other for it.

use chrono::{DateTime, Utc};

use auth::{ApiKey, ApiKeyId};

use crate::api::api_key_hash::hash_api_key;

/// Build a new ApiKey struct and return (plaintext, model).
pub(super) fn build_api_key(
    name: &str,
    user_id: auth::UserId,
    expires_at: Option<DateTime<Utc>>,
) -> (String, ApiKey) {
    let raw_key = format!(
        "ak_{}_{}",
        generate_key_segment(4),
        generate_key_segment(32)
    );
    let key_prefix = format!("{}****{}", &raw_key[..8], &raw_key[raw_key.len() - 4..]);
    let key_hash = hash_api_key(&raw_key);
    let now = Utc::now();

    let api_key = ApiKey {
        id: ApiKeyId::new(),
        user_id,
        name: name.to_string(),
        key_hash,
        key_prefix,
        is_active: true,
        created_at: now,
        last_used_at: None,
        expires_at,
    };

    (raw_key, api_key)
}

/// Generate a random hex segment. Used both for the raw API key itself and
/// for a wallet reauth-challenge token.
pub(super) fn generate_key_segment(bytes: usize) -> String {
    use std::fmt::Write;
    let mut buf = vec![0u8; bytes];
    // getrandom only fails if the system has no RNG. If that's happened we
    // have much bigger problems than this function — surface it and crash
    // cleanly rather than silently minting a zero-entropy key.
    if getrandom::fill(&mut buf).is_err() {
        // Zeroed buffer would be a catastrophic key; panic here rather than
        // return weak entropy. This is an init-time invariant.
        #[allow(clippy::panic, reason = "system RNG missing is unrecoverable")]
        {
            panic!("getrandom failed — refusing to mint weak API key");
        }
    }
    let mut s = String::with_capacity(bytes * 2);
    for b in &buf {
        // write! on String can only fail on OOM — fmt::Write for String is
        // infallible in practice.
        #[allow(clippy::unwrap_used, reason = "writing to String is infallible")]
        write!(s, "{:02x}", b).unwrap();
    }
    s
}
