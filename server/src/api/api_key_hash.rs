//! SHA-256 hashing for API keys.
//!
//! Shared between `extractors::validate_api_key` (auth path) and
//! `users::build_api_key` (creation path) so both paths can never drift.

/// Hash an API key with SHA-256 and return hex-encoded output.
pub(super) fn hash_api_key(raw_key: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(raw_key.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_are_deterministic() {
        let a = hash_api_key("ak_abcd_1234567890");
        let b = hash_api_key("ak_abcd_1234567890");
        assert_eq!(a, b);
    }

    #[test]
    fn different_inputs_produce_different_hashes() {
        let a = hash_api_key("ak_aaa");
        let b = hash_api_key("ak_bbb");
        assert_ne!(a, b);
    }

    #[test]
    fn output_is_64_hex_chars() {
        let h = hash_api_key("ak_test");
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
