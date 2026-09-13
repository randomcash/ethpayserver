//! Server-wide reads of which merchants exist (RCS-300).
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

#[cfg(test)]
mod tests {
    use std::sync::RwLock;

    use uuid::Uuid;

    use super::*;

    /// A directory backed by a lock, standing in for `PgDataService` here so
    /// the ownership property below needs no database.
    struct FakeDirectory {
        stores: RwLock<Vec<MerchantStore>>,
    }

    #[async_trait]
    impl MerchantDirectoryReader for FakeDirectory {
        async fn list_accounts(
            &self,
            _offset: i64,
            _limit: i64,
        ) -> RepositoryResult<Vec<MerchantAccount>> {
            Ok(Vec::new())
        }

        async fn list_stores(
            &self,
            _offset: i64,
            _limit: i64,
        ) -> RepositoryResult<Vec<MerchantStore>> {
            Ok(self
                .stores
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .clone())
        }
    }

    /// Ticket test 4: a plugin reading core data gets owned values, and
    /// cannot mutate anything through them.
    ///
    /// `list_stores` returns `Vec<MerchantStore>` by value, not a reference
    /// into the directory's own storage. Mutating what the caller got back
    /// must therefore be inert: a second read has to come back unchanged.
    #[tokio::test]
    async fn mutating_the_returned_list_does_not_touch_the_source() {
        let directory = FakeDirectory {
            stores: RwLock::new(vec![MerchantStore {
                id: StoreId(Uuid::new_v4()),
                name: "original".to_string(),
                owner_id: UserId(Uuid::new_v4()),
                archived: false,
            }]),
        };

        let mut first_read = directory.list_stores(0, 10).await.expect("list stores");
        first_read[0].name = "tampered".to_string();
        first_read[0].archived = true;

        let second_read = directory.list_stores(0, 10).await.expect("list stores");
        assert_eq!(second_read[0].name, "original");
        assert!(!second_read[0].archived);
    }
}
