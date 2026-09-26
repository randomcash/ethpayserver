//! Capability 8: tell every plugin an account is gone.
//!
//! The mirror image of `payment_observer`'s capability 4: told after the
//! fact, returns nothing, cannot refuse. Deletion cascades the host's own
//! tables and stops at the schema boundary schema-per-plugin storage exists
//! to enforce - a plugin's rows in its own schema are its business, not
//! something the host can reach with a `DELETE`. Without this, a plugin has
//! no way to learn an account it holds data for no longer exists, and that
//! data outlives the account forever.
//!
//! Unfiltered by kind, the same as [`super::payment_observers`]: a plugin
//! that does not export `account_closed` simply records a failure and
//! nothing else happens, because there is no verdict to resolve and no
//! caller waiting on an answer.

use std::sync::Arc;

use async_trait::async_trait;
use auth::UserId;

/// Something that wants to know an account no longer exists.
///
/// Returns `()`: an account is already gone by the time this is called, so
/// there is nothing left for an implementation to refuse or delay.
#[async_trait]
pub trait AccountClosedObserver: Send + Sync {
    async fn account_closed(&self, account_id: UserId);
}

/// Tell every observer that `account_id` no longer exists.
///
/// Called after the account's own deletion is committed - see the callers -
/// so this can never stand between a merchant and having their account
/// deleted. Observers are awaited in order, the same tradeoff
/// `notify_own_store_payment` makes: the caller is already past the write
/// that matters, and bounding a plugin's call time is the runtime's job.
pub async fn notify_account_closed(
    observers: &[Arc<dyn AccountClosedObserver>],
    account_id: UserId,
) {
    for observer in observers {
        observer.account_closed(account_id).await;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::Mutex;

    use uuid::Uuid;

    use super::*;

    #[derive(Default)]
    struct Recorder {
        seen: Mutex<Vec<UserId>>,
    }

    #[async_trait]
    impl AccountClosedObserver for Recorder {
        async fn account_closed(&self, account_id: UserId) {
            self.seen.lock().unwrap().push(account_id);
        }
    }

    #[tokio::test]
    async fn every_observer_is_told() {
        let account_id = UserId(Uuid::new_v4());
        let first = Arc::new(Recorder::default());
        let second = Arc::new(Recorder::default());
        let observers: Vec<Arc<dyn AccountClosedObserver>> = vec![first.clone(), second.clone()];

        notify_account_closed(&observers, account_id).await;

        assert_eq!(first.seen.lock().unwrap().as_slice(), [account_id]);
        assert_eq!(second.seen.lock().unwrap().as_slice(), [account_id]);
    }

    #[tokio::test]
    async fn no_observers_is_not_an_error() {
        notify_account_closed(&[], UserId(Uuid::new_v4())).await;
    }
}
