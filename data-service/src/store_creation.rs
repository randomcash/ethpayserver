//! Creating a store as one unit of work.
//!
//! Creating a store writes two rows: the store itself, and the `user_stores`
//! membership that makes its creator the owner. Those used to be two independent
//! trips to the database with a role lookup between them, each committing on its
//! own.
//!
//! When the lookup or the membership write failed, the caller got a 500 and the
//! `stores` row stayed behind — owned by nobody, because the membership is what
//! ownership means here. The UI lists stores by membership, so the row is
//! invisible in the product and cannot be deleted through it. It was observed
//! directly: every creation had returned 500, and every row was still in
//! `stores`.
//!
//! # The failure that surfaced it
//!
//! `get_default_role_by_name("Owner")` returned `None`, because the four global
//! default roles seeded by migration `20241214000001` had been deleted. That
//! particular cause is fixed elsewhere, but the same lookup returns `None` on any
//! deployment whose seed migration did not run — and a missing seed is a
//! deployment fault, not a reason to leave an unowned row behind.
//!
//! So the role lookup is inside the unit of work too: if there is no Owner role,
//! nothing is written at all.

use async_trait::async_trait;
use auth::error::AuthError;
use auth::{Store, UserStore};

/// Why a store could not be created.
///
/// The handler previously mapped all five failure paths to a bare 500 with `|_|`,
/// discarding the error, and logged nothing — the only trace was
/// `tower_http ... classification=Status code: 500`. Working out which of the
/// five had happened took a database inspection.
#[derive(Debug)]
pub enum StoreCreationError {
    /// No default role named "Owner" exists.
    ///
    /// Distinguished from a repository error on purpose: this one means the
    /// deployment's seed data is missing or was removed, which is an operator
    /// problem with a specific fix, not a transient database fault.
    MissingOwnerRole,
    /// The database refused one of the writes, or the lookup itself failed.
    Repository(AuthError),
}

impl std::fmt::Display for StoreCreationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreCreationError::MissingOwnerRole => write!(
                f,
                "no default store role named 'Owner' — the seed migration has not run \
                 or its rows were deleted"
            ),
            StoreCreationError::Repository(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for StoreCreationError {}

impl From<AuthError> for StoreCreationError {
    fn from(e: AuthError) -> Self {
        StoreCreationError::Repository(e)
    }
}

/// Write a store and the membership that owns it, or write neither.
#[async_trait]
pub trait StoreCreationWriter: Send + Sync {
    /// Insert `store`, resolve the default Owner role, and record `owner_id` as a
    /// member holding it — in a single transaction.
    ///
    /// On any error nothing is written, including the store.
    async fn create_store_owned_by(
        &self,
        store: &Store,
        owner_id: auth::UserId,
    ) -> Result<UserStore, StoreCreationError>;
}
