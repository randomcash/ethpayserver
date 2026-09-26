//! Hard-deleting an account or a store.
//!
//! Split out of `admin::mod` (which was crossing the repo's 400-line file
//! limit) along the two endpoints it covers: `account` is
//! `DELETE /admin/users/{id}`, `store` is `DELETE /admin/stores/{id}`. Both
//! read a target's still-active watched addresses before their cascading
//! delete removes the rows those addresses point at - `account`'s read (and
//! self-service `delete_account`'s, in `api::users`, which shares it) is only
//! ever a refuse-if-nonempty gate, while `store`'s hands the result to a
//! post-delete unwatch step, since unwatching a synthetic E2E store on
//! purpose is the whole point there. That shared read is [`active_watched_addresses`].

use evm::Address;

use data_service::CleanupAddressInfo;

use crate::services::evm_monitor::EVMMonitor;
use crate::state::PgAppState;
use auth::SessionService;
use axum::http::StatusCode;

pub(crate) mod account;
pub(crate) mod store;

pub use account::{delete_user_account, list_user_stores};
pub use store::{E2E_STORE_OWNER_ID, hard_delete_store};

/// Look up every still-active watched address for invoices under
/// `store_ids`, before their stores are removed.
///
/// `account_deletion_blockers` (and, for `hard_delete_store`, the
/// payout/refund check below) only count *recorded* financial history - an
/// invoice with an address generated and watched, but no payment recorded
/// yet, passes both untouched. Read here, before the delete, because the
/// delete's cascade removes these very rows - by the time it has run there is
/// nothing left to look up. What to do with the result is `unwatch_after_delete`'s
/// job, not this function's: this only reads.
///
/// `pub(crate)` rather than private: self-service `DELETE /users/me`
/// (`api::users::delete_account`) cascades through the same owned stores and
/// needs the identical guard - `account_deletion_blockers` only sees
/// *recorded* payments there too, so without this check a merchant could
/// delete their own account out from under a pending, unconfirmed payment
/// exactly as an admin-driven delete could before this module existed.
pub(crate) async fn active_watched_addresses<A>(
    state: &PgAppState<A>,
    store_ids: &[uuid::Uuid],
) -> Result<Vec<CleanupAddressInfo>, (StatusCode, String)>
where
    A: SessionService + 'static,
{
    state
        .data_service
        .get_active_watched_addresses_for_stores(store_ids)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not look up watched addresses.".to_string(),
            )
        })
}

/// Tell the monitor to stop watching `addresses`, once the delete that made
/// them stale has already succeeded - never before.
///
/// An earlier version of this ran the unwatch step *before* the delete, to
/// close the gap the ticket calls out by name: a no-TTL Redis key pointing at
/// an invoice id that no longer exists, left behind because deleting straight
/// through a still-pending, still-watched invoice never told the monitor to
/// stop. That ordering opened a worse gap of its own: unwatching is an
/// external side effect with no rollback, so a delete that was then refused
/// (a payment landing in the window between the two steps, or any other
/// failure) left a still-*live* invoice unwatched - the account survived, but
/// the monitor had already been told to stop polling it. Run only after a
/// delete that has already committed, the reverse is what happens instead: a
/// failure here can at worst reproduce the original trap (the monitor keeps
/// polling a now-deleted invoice a little longer), never destroy the watch on
/// one that still exists. Best-effort and logged rather than propagated for
/// that same reason - the delete this follows already succeeded, and a
/// network error talking to the monitor must not turn that into a reported
/// failure.
///
/// Only `hard_delete_store` calls this: `delete_user_account` and self-service
/// `delete_account` both refuse outright on a still-watched address rather
/// than unwatching and proceeding, so `addresses` is never non-empty by the
/// time either of them would reach a call here.
/// Counts failures of the post-delete unwatch.
///
/// Deliberately defined here rather than in `metrics.rs`, which is 169 lines
/// over the file-size limit - the gate refuses any addition to it, so a new
/// counter cannot go in the module that holds every other one. Move it there
/// when that file is split.
///
/// Why it exists: the deletion path used to make orphaned watches impossible
/// by refusing the delete while any address was watched. That refusal was
/// broader than its purpose and is gone, so the watches are cleared after the
/// delete commits instead - best effort rather than guaranteed. This is how we
/// find out which of those we actually have. If it never moves, the gap is
/// theoretical; if it does, a stale watch exists and the reconciler is not
/// optional.
fn record_unwatch_failed() {
    metrics::counter!("ethpayserver_unwatch_after_delete_failures_total").increment(1);
}

pub(crate) async fn unwatch_after_delete<A>(
    state: &PgAppState<A>,
    addresses: Vec<CleanupAddressInfo>,
) where
    A: SessionService + 'static,
{
    let Some(monitor) = &state.evm_monitor else {
        // Assumes watching and unwatching always go through the same
        // process's monitor handle - true for every deployment shape this
        // runs in today, but not something this function can verify. Logged
        // rather than silently skipped so that assumption failing anywhere
        // is at least observable instead of indistinguishable from a normal
        // no-op unwatch.
        if !addresses.is_empty() {
            tracing::warn!(
                count = addresses.len(),
                "no live monitor wired into this process; skipped unwatching \
                 address(es) after delete - if a separate process is watching \
                 them via Redis, the monitor may keep polling a deleted invoice",
            );
            // Counted, not just logged. This branch leaves exactly the stale
            // watch the counter exists to detect, and a warning nobody greps
            // is not detection.
            for _ in 0..addresses.len() {
                record_unwatch_failed();
            }
        }
        return;
    };

    for info in addresses {
        let Some((addr, token_contract, eip155)) = parse_watch_target(&info) else {
            continue;
        };

        if let Err(e) = monitor
            .unwatch_address_by_chain_id(eip155, addr, token_contract)
            .await
        {
            tracing::error!(
                address = %info.address,
                chain_id = eip155,
                error = %e,
                "could not unwatch address after delete - the monitor may keep \
                 polling a deleted invoice",
            );
            record_unwatch_failed();
        }
    }
}

/// Parse a `CleanupAddressInfo` row into what `unwatch_after_delete` needs to
/// call the monitor, logging and returning `None` on anything malformed
/// rather than letting one bad row stop the rest of the batch.
fn parse_watch_target(info: &CleanupAddressInfo) -> Option<(Address, Option<Address>, u64)> {
    let Ok(addr) = info.address.parse::<Address>() else {
        tracing::error!(
            address = %info.address,
            "watched address is not valid; could not unwatch it after delete - \
             the monitor may keep polling a deleted invoice",
        );
        return None;
    };
    // A malformed value here must not be treated as "no token" - that would
    // unwatch the native-asset entry instead of the ERC20 one, leaving the
    // real watch live with nothing to show for it.
    let token_contract: Option<Address> = match info.token_address.as_deref() {
        Some(t) => match t.parse() {
            Ok(a) => Some(a),
            Err(_) => {
                tracing::error!(
                    token_address = t,
                    address = %info.address,
                    "token address is not valid; could not unwatch it after delete - \
                     the monitor may keep polling a deleted invoice",
                );
                return None;
            }
        },
        None => None,
    };
    let Some(eip155) = info.chain_id.evm_chain_id() else {
        tracing::error!(
            chain_id = %info.chain_id,
            address = %info.address,
            "not an EVM chain; could not unwatch after delete - the monitor may \
             keep polling a deleted invoice",
        );
        return None;
    };

    Some((addr, token_contract, eip155))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use types::ChainId;

    fn cleanup_info(
        address: &str,
        token_address: Option<&str>,
        chain_id: ChainId,
    ) -> CleanupAddressInfo {
        CleanupAddressInfo {
            address: address.to_string(),
            payment_option_id: data_service::PaymentOptionId::new(),
            invoice_id: "inv-test".to_string(),
            chain_id,
            token_address: token_address.map(str::to_string),
        }
    }

    /// The one branch a malformed watch row must not fall into: an unparsable
    /// token address is not the same as no token address, and treating it as
    /// one would unwatch the native-asset entry instead of the ERC20 one,
    /// leaving the real watch live under a reported success.
    #[test]
    fn parse_watch_target_rejects_a_malformed_token_address_instead_of_treating_it_as_none() {
        let info = cleanup_info(
            "0x1111111111111111111111111111111111111111",
            Some("not-an-address"),
            ChainId::evm(11155111),
        );
        assert_eq!(parse_watch_target(&info), None);
    }

    #[test]
    fn parse_watch_target_rejects_a_malformed_address() {
        let info = cleanup_info("not-an-address", None, ChainId::evm(11155111));
        assert_eq!(parse_watch_target(&info), None);
    }

    #[test]
    fn parse_watch_target_rejects_a_non_evm_chain() {
        let info = cleanup_info(
            "0x1111111111111111111111111111111111111111",
            None,
            ChainId::new("tron", "728126428").expect("valid chain id"),
        );
        assert_eq!(parse_watch_target(&info), None);
    }

    #[test]
    fn parse_watch_target_accepts_a_well_formed_native_watch() {
        let info = cleanup_info(
            "0x1111111111111111111111111111111111111111",
            None,
            ChainId::evm(11155111),
        );
        let (addr, token, eip155) = parse_watch_target(&info).expect("should parse");
        let expected: Address = "0x1111111111111111111111111111111111111111"
            .parse()
            .expect("valid address");
        assert_eq!(addr, expected);
        assert_eq!(token, None);
        assert_eq!(eip155, 11155111);
    }
}
