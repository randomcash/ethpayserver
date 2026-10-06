//! Pending email changes awaiting verification.
//!
//! A merchant changing their account email must prove they can read mail at
//! the new address before the account record moves - otherwise a typo or a
//! hijacked session leaves the account pointed at an address nobody can
//! confirm, silently. This module is the storage for that proof: a single-use,
//! short-lived token tied to one pending `(user, new_email)` pair.
//!
//! `kdf_salt_identifier` is untouched by any of this - it is set once at
//! registration and never changes, so a confirmed email change is applied via
//! the ordinary `auth::UserRepository::update_user` path against the fetched
//! `User`, which already tolerates changing `email` alone.

use async_trait::async_trait;
use auth::UserId;
use chrono::{DateTime, Utc};
use types::RepositoryResult;
use uuid::Uuid;

/// A pending, unconfirmed email change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailChangeRequest {
    pub token: Uuid,
    pub user_id: UserId,
    pub new_email: String,
    pub expires_at: DateTime<Utc>,
}

/// Create and redeem pending email-change tokens.
#[async_trait]
pub trait EmailChangeWriter: Send + Sync {
    /// Start a new pending change, superseding any prior unconsumed request
    /// for this user.
    ///
    /// Superseding rather than merely inserting alongside is deliberate: a
    /// merchant who mistypes an address and requests again should not leave
    /// the first, wrong token still redeemable - and only one pending change
    /// should ever be able to land.
    async fn create_email_change_request(
        &self,
        user_id: UserId,
        new_email: &str,
        expires_at: DateTime<Utc>,
    ) -> RepositoryResult<EmailChangeRequest>;

    /// Redeem a token if it exists, is unexpired and has not already been
    /// used. Returns `None` for a missing, expired or already-consumed token
    /// without distinguishing which - a caller building an error message for
    /// the confirming user has no legitimate use for telling those apart, and
    /// distinguishing them would tell an attacker guessing tokens which part
    /// of the guess was close.
    ///
    /// The consume is atomic: two concurrent confirmations of the same token
    /// can only ever have one winner.
    async fn consume_email_change_request(
        &self,
        token: Uuid,
    ) -> RepositoryResult<Option<EmailChangeRequest>>;
}
