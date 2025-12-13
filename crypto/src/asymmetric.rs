//! Asymmetric cryptography: X25519 key exchange and Ed25519 signing.
//!
//! - X25519 (RFC 7748): Elliptic curve Diffie-Hellman for key exchange
//! - Ed25519 (RFC 8032): Edwards-curve signatures

#![allow(unused_assignments)] // False positives from zeroize derive macro

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::error::CryptoError;

/// X25519 key pair for Diffie-Hellman key exchange.
///
/// Implements ZeroizeOnDrop to ensure secret key material is cleared from memory.
/// (StaticSecret only implements Zeroize, not ZeroizeOnDrop, so we must derive it.)
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct X25519KeyPair {
    #[zeroize(skip)] // PublicKey is public, doesn't need zeroizing
    pub public_key: X25519PublicKey,
    secret_key: StaticSecret,
}

impl X25519KeyPair {
    /// Generate a new random X25519 key pair.
    pub fn generate() -> Self {
        let secret_key = StaticSecret::random_from_rng(OsRng);
        let public_key = X25519PublicKey::from(&secret_key);
        Self {
            public_key,
            secret_key,
        }
    }

    /// Create from a 32-byte secret key.
    pub fn from_secret(secret: &[u8; 32]) -> Self {
        let secret_key = StaticSecret::from(*secret);
        let public_key = X25519PublicKey::from(&secret_key);
        Self {
            public_key,
            secret_key,
        }
    }

    /// Get the public key bytes.
    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.public_key.to_bytes()
    }

    /// Perform Diffie-Hellman key exchange with another party's public key.
    ///
    /// Returns a 32-byte shared secret.
    ///
    /// # Security
    /// The returned shared secret is sensitive key material. Caller MUST
    /// zeroize it after use (e.g., by wrapping in SymmetricKey or using Zeroize).
    pub fn diffie_hellman(&self, their_public: &[u8; 32]) -> [u8; 32] {
        let their_public = X25519PublicKey::from(*their_public);
        self.secret_key.diffie_hellman(&their_public).to_bytes()
    }
}

/// Ed25519 key pair for digital signatures.
///
/// Note: SigningKey from ed25519-dalek implements ZeroizeOnDrop internally,
/// so the secret key material is automatically cleared when this struct is dropped.
pub struct Ed25519KeyPair {
    signing_key: SigningKey,
}

impl Ed25519KeyPair {
    /// Generate a new random Ed25519 key pair.
    pub fn generate() -> Self {
        let signing_key = SigningKey::generate(&mut OsRng);
        Self { signing_key }
    }

    /// Create from a 32-byte seed.
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        let signing_key = SigningKey::from_bytes(seed);
        Self { signing_key }
    }

    /// Get the public (verifying) key bytes.
    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }

    /// Get the verifying key for signature verification.
    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    /// Sign a message.
    ///
    /// Returns a 64-byte Ed25519 signature.
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.signing_key.sign(message).to_bytes()
    }

    /// Verify a signature.
    pub fn verify(&self, message: &[u8], signature: &[u8; 64]) -> Result<(), CryptoError> {
        let sig = Signature::from_bytes(signature);
        self.signing_key
            .verifying_key()
            .verify(message, &sig)
            .map_err(|_| CryptoError::InvalidSignature)
    }
}

/// Verify an Ed25519 signature with just the public key.
pub fn verify_signature(
    public_key: &[u8; 32],
    message: &[u8],
    signature: &[u8; 64],
) -> Result<(), CryptoError> {
    let verifying_key = VerifyingKey::from_bytes(public_key)
        .map_err(|_| CryptoError::InvalidSignature)?;
    let sig = Signature::from_bytes(signature);
    verifying_key
        .verify(message, &sig)
        .map_err(|_| CryptoError::InvalidSignature)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_x25519_key_generation() {
        let kp1 = X25519KeyPair::generate();
        let kp2 = X25519KeyPair::generate();

        // Different key pairs should have different public keys
        assert_ne!(kp1.public_key_bytes(), kp2.public_key_bytes());
    }

    #[test]
    fn test_x25519_from_secret() {
        let secret = [42u8; 32];
        let kp1 = X25519KeyPair::from_secret(&secret);
        let kp2 = X25519KeyPair::from_secret(&secret);

        // Same secret should produce same public key
        assert_eq!(kp1.public_key_bytes(), kp2.public_key_bytes());
    }

    #[test]
    fn test_x25519_diffie_hellman() {
        let alice = X25519KeyPair::generate();
        let bob = X25519KeyPair::generate();

        // Both parties should derive the same shared secret
        let alice_shared = alice.diffie_hellman(&bob.public_key_bytes());
        let bob_shared = bob.diffie_hellman(&alice.public_key_bytes());

        assert_eq!(alice_shared, bob_shared);
    }

    #[test]
    fn test_ed25519_key_generation() {
        let kp1 = Ed25519KeyPair::generate();
        let kp2 = Ed25519KeyPair::generate();

        // Different key pairs should have different public keys
        assert_ne!(kp1.public_key_bytes(), kp2.public_key_bytes());
    }

    #[test]
    fn test_ed25519_from_seed() {
        let seed = [42u8; 32];
        let kp1 = Ed25519KeyPair::from_seed(&seed);
        let kp2 = Ed25519KeyPair::from_seed(&seed);

        // Same seed should produce same public key
        assert_eq!(kp1.public_key_bytes(), kp2.public_key_bytes());
    }

    #[test]
    fn test_ed25519_sign_verify() {
        let kp = Ed25519KeyPair::generate();
        let message = b"Hello, World!";

        let signature = kp.sign(message);
        assert!(kp.verify(message, &signature).is_ok());
    }

    #[test]
    fn test_ed25519_verify_with_public_key() {
        let kp = Ed25519KeyPair::generate();
        let message = b"Test message";

        let signature = kp.sign(message);
        let public_key = kp.public_key_bytes();

        assert!(verify_signature(&public_key, message, &signature).is_ok());
    }

    #[test]
    fn test_ed25519_wrong_message_fails() {
        let kp = Ed25519KeyPair::generate();
        let message = b"Original message";
        let wrong_message = b"Wrong message";

        let signature = kp.sign(message);
        assert!(kp.verify(wrong_message, &signature).is_err());
    }

    #[test]
    fn test_ed25519_wrong_signature_fails() {
        let kp = Ed25519KeyPair::generate();
        let message = b"Test message";

        let mut signature = kp.sign(message);
        signature[0] ^= 0xff; // Tamper with signature

        assert!(kp.verify(message, &signature).is_err());
    }

    #[test]
    fn test_ed25519_wrong_key_fails() {
        let kp1 = Ed25519KeyPair::generate();
        let kp2 = Ed25519KeyPair::generate();
        let message = b"Test message";

        let signature = kp1.sign(message);

        // Verify with wrong key should fail
        assert!(kp2.verify(message, &signature).is_err());
    }

    #[test]
    fn test_ed25519_deterministic_signatures() {
        let kp = Ed25519KeyPair::from_seed(&[42u8; 32]);
        let message = b"Same message";

        let sig1 = kp.sign(message);
        let sig2 = kp.sign(message);

        // Ed25519 signatures are deterministic
        assert_eq!(sig1, sig2);
    }
}
