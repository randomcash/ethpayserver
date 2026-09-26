#![allow(clippy::unwrap_used)]
use super::*;

// Deterministic 32-byte private key, matching the pattern the auth
// crate's own `test_wallet_signature_verification` uses — not a real
// key, chosen only so both the test and a would-be attacker can derive
// the same address from it.
const TEST_PRIVATE_KEY: [u8; 32] = [
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10,
    0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x20,
];

fn test_signing_key() -> k256::ecdsa::SigningKey {
    k256::ecdsa::SigningKey::from_bytes((&TEST_PRIVATE_KEY).into()).unwrap()
}

fn address_for(signing_key: &k256::ecdsa::SigningKey) -> String {
    let verifying_key = signing_key.verifying_key();
    let public_key_bytes = verifying_key.to_encoded_point(false);
    let public_key_hash = sha3::Keccak256::digest(&public_key_bytes.as_bytes()[1..]);
    format!("0x{}", hex::encode(&public_key_hash[12..]))
}

fn sign(signing_key: &k256::ecdsa::SigningKey, message: &str) -> String {
    let prefix = format!("\x19Ethereum Signed Message:\n{}", message.len());
    let mut hasher = sha3::Keccak256::new();
    hasher.update(prefix.as_bytes());
    hasher.update(message.as_bytes());
    let message_hash = hasher.finalize();

    let (signature, recovery_id) = signing_key.sign_prehash_recoverable(&message_hash).unwrap();
    let mut sig_bytes = signature.to_bytes().to_vec();
    sig_bytes.push(recovery_id.to_byte() + 27);
    format!("0x{}", hex::encode(&sig_bytes))
}

#[test]
fn a_correctly_signed_challenge_verifies() {
    let key = test_signing_key();
    let address = address_for(&key);
    let message = wallet_reauth_challenge_message("deadbeef", &address, Utc::now());
    let signature = sign(&key, &message);

    assert!(verify_wallet_signature(&message, &signature, &address));
}

#[test]
fn a_signature_over_a_different_message_does_not_verify() {
    let key = test_signing_key();
    let address = address_for(&key);
    let message = wallet_reauth_challenge_message("deadbeef", &address, Utc::now());
    let signature = sign(&key, &message);

    let tampered = wallet_reauth_challenge_message("deadc0de", &address, Utc::now());
    assert!(!verify_wallet_signature(&tampered, &signature, &address));
}

#[test]
fn a_signature_from_a_different_key_does_not_verify() {
    let key = test_signing_key();
    let address = address_for(&key);
    let message = wallet_reauth_challenge_message("deadbeef", &address, Utc::now());
    let signature = sign(&key, &message);

    // Someone who hijacked the session but does not hold the wallet's
    // private key cannot produce a signature that recovers to it — this
    // is the property that makes the challenge a genuine step-up rather
    // than just a session check.
    let other_address = "0x000000000000000000000000000000000000ff";
    assert!(!verify_wallet_signature(
        &message,
        &signature,
        other_address
    ));
}

#[test]
fn garbage_signature_hex_does_not_verify() {
    let message = wallet_reauth_challenge_message("deadbeef", "0xabc", Utc::now());
    assert!(!verify_wallet_signature(&message, "not-hex", "0xabc"));
    assert!(!verify_wallet_signature(&message, "0x1234", "0xabc"));
}
