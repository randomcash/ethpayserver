//! Server-wide reads of which merchants exist.
//!
//! `UserRepository` and `StoreRepository` (both in `auth`, pinned via the
//! commons dance) can list *a* user's stores or count *all* users for the
//! admin page, but neither can list every store on the instance - nothing
//! before this needed to ask "every merchant, regardless of who is looking."
//! The plugin host does: a billing plugin has no session and no store of its
//! own to scope from, so it needs the server-wide answer directly.
//!
//! This lives here, in this server's own `data-service` crate, rather than as
//! a new method on `auth`'s traits - adding one there would need the
//! three-step commons dance (change, merge, bump the pin) before any of this
//! could compile, for a query that is entirely about this server's own
//! `users`/`stores` tables.

use async_trait::async_trait;
use auth::UserId;
use chrono::{DateTime, Utc};
use types::{RepositoryResult, StoreId};

/// One merchant account, as far as a plugin is ever allowed to see it.
///
/// Deliberately not `auth::User`: that type carries KDF parameters, an
/// encrypted symmetric key and a recovery hash. A plugin gets an identifier
/// and nothing that would let it authenticate as, or recover, the account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerchantAccount {
    pub id: UserId,
    pub created_at: DateTime<Utc>,
}

/// One store, as far as a plugin is ever allowed to see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerchantStore {
    pub id: StoreId,
    pub name: String,
    pub owner_id: UserId,
    pub archived: bool,
}

/// Server-wide, read-only directory of merchants.
///
/// Deliberately narrow: no lookup by email or wallet address, no store
/// settings, no payment history - just the two lists and the identifiers on
/// them, which is all billing needs to know an account or a store exists.
///
/// Ticket test 4's mutation-immunity half ("cannot mutate anything through
/// them") is not pinned by a runtime test: both return types are owned
/// (`Vec<MerchantAccount>` / `Vec<MerchantStore>` of plain `Uuid`/`String`/
/// `bool`/`DateTime` fields, no `Arc`, `Rc`, or interior mutability), so no
/// implementation matching this signature can expose a handle back into its
/// own storage - the compiler rejects anything that would. A runtime test
/// that mutates a returned `Vec` and reads again cannot fail for any
/// type-correct implementation, which is worse than no test: it looks like
/// coverage without providing any. The reading-correctness half of test 4 is
/// covered by the postgres integration test instead, where an implementation
/// can actually be wrong.
#[async_trait]
pub trait MerchantDirectoryReader: Send + Sync {
    /// Every account on this instance, oldest first. Mirrors
    /// `UserRepository::list_users`'s `(offset, limit)` shape.
    async fn list_accounts(
        &self,
        offset: i64,
        limit: i64,
    ) -> RepositoryResult<Vec<MerchantAccount>>;

    /// Every store on this instance, oldest first.
    async fn list_stores(&self, offset: i64, limit: i64) -> RepositoryResult<Vec<MerchantStore>>;
}
