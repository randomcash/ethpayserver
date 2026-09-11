//! What stands in the way of deleting an account.
//!
//! `users` cascades hard. Traced against the live schema:
//!
//! ```text
//! users -> stores -> invoices -> payments        (all ON DELETE CASCADE)
//!                 -> payouts                     (NO ACTION)
//!                 -> refunds                     (NO ACTION)
//! ```
//!
//! So a bare `DELETE FROM users` does one of two bad things. For an account
//! whose stores took money it **silently destroys the payment history** - the
//! rows a merchant would need to answer a customer, a chargeback or a tax
//! question. For an account with a payout or a refund it fails on a foreign key
//! instead, surfacing as an opaque 500.
//!
//! Neither is a decision anyone made; both fall out of the schema. This module
//! is the deliberate answer: count what would be destroyed first, and let the
//! caller refuse.
//!
//! Deleting is only allowed for an account with no financial history at all.
//! That covers the cases deletion is actually for - an abandoned signup, a test
//! account, a merchant who never traded - and refuses the one case where the
//! damage is unrecoverable. `DELETE /stores/{id}` already sets this precedent by
//! archiving rather than deleting.

use async_trait::async_trait;
use auth::UserId;
use types::RepositoryResult;

/// Financial records that would be destroyed by deleting an account.
///
/// Counts, not booleans: a merchant told "you have payments" will ask how many,
/// and an operator triaging a refusal needs to know which table is holding it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AccountDeletionBlockers {
    /// Payments recorded against invoices in stores this account owns.
    pub payments: i64,
    /// Payouts from those stores.
    pub payouts: i64,
    /// Refunds against payments in those stores.
    pub refunds: i64,
}

impl AccountDeletionBlockers {
    /// Whether anything at all stands in the way.
    #[must_use]
    pub fn any(&self) -> bool {
        self.payments > 0 || self.payouts > 0 || self.refunds > 0
    }

    /// A sentence naming what is blocking, for the merchant to read.
    ///
    /// Empty when nothing blocks.
    #[must_use]
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if self.payments > 0 {
            parts.push(format!("{} payment(s)", self.payments));
        }
        if self.payouts > 0 {
            parts.push(format!("{} payout(s)", self.payouts));
        }
        if self.refunds > 0 {
            parts.push(format!("{} refund(s)", self.refunds));
        }
        parts.join(", ")
    }
}

/// Read what would be lost if this account were deleted.
#[async_trait]
pub trait AccountDeletionReader: Send + Sync {
    /// Count the financial records held by stores this account owns.
    ///
    /// Ownership is `stores.owner_id`, which is the column that cascades.
    /// Membership of someone else's store is deliberately not counted: deleting
    /// this account does not touch their data.
    async fn account_deletion_blockers(
        &self,
        user_id: UserId,
    ) -> RepositoryResult<AccountDeletionBlockers>;
}
