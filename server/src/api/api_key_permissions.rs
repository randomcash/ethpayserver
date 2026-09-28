//! Validating the permission scope requested for an API key.
//!
//! Split out of `users` because the request validation and its unit tests
//! were a sizeable share of that file. The scope a key is created with is
//! checked here; there is deliberately no endpoint for editing an existing
//! key's scope after the fact - see the ticket referenced in the commit that
//! removed it.

use axum::http::StatusCode;
use uuid::Uuid;

use auth::{Permission, Policies, Role};

/// Is a single requested permission entry a store-scoped grant this server
/// can actually enforce?
///
/// `Policies::is_store_policy` names an action `user_has_store_permission`
/// checks directly in SQL - real enforcement, unlike the server/user
/// policies `validate_requested_permissions` still refuses below. Optionally
/// suffixed with `:<storeId>` (`key_grants_store_permission` in
/// `extractors.rs` is the other half that reads it) to narrow the grant to
/// one store rather than every store the owner can reach; a malformed
/// suffix is rejected outright rather than silently falling back to
/// "every store" - a typo should fail the request, not widen it.
fn is_store_scope_entry(entry: &str) -> bool {
    match entry.split_once(':') {
        Some((policy, store_id)) => {
            Policies::is_store_policy(policy) && Uuid::parse_str(store_id).is_ok()
        }
        None => Policies::is_store_policy(entry),
    }
}

/// Is `requested` a scope this server can actually enforce?
///
/// Every *admin* gate in this codebase (there are over a dozen, from plugin
/// install to user role management) is a bare `role == Role::ServerAdmin`
/// comparison, not a check against an individual `Permission` - see
/// `validate_api_key`'s downgrade and `key_retains_unrestricted_access` in
/// `extractors.rs`. So a stored set naming specific server or user policies
/// (e.g. just `ethpay.server.canmanagetokens`) would be silently
/// indistinguishable from an empty one: neither grants anything past a plain
/// `User`'s fixed permissions. This still refuses those: nothing beyond the
/// owner's non-admin baseline (`[]`) or the owner's full role
/// (`["unrestricted"]`) for that half.
///
/// Store permissions are different: `user_has_store_permission` already
/// checks them individually, in SQL, at real call sites (invoice creation,
/// store settings, store membership). A key naming one or more of those
/// (`is_store_scope_entry`) is accepted regardless of `owner_role` - the
/// grant is never wider than what `user_has_store_permission` would allow
/// the owner anyway, since enforcement always intersects the two (see
/// `key_grants_store_permission`), so there is nothing here to launder.
///
/// `owner_role` is always the role belonging to the account the key
/// authenticates as, never the caller's - the write-time half of "a key can
/// never exceed its owner". Read-time enforcement (`validate_api_key`, and
/// every `role == Role::ServerAdmin` check downstream of it) covers a key
/// whose owner's role later changes, but without this an admin editing
/// someone else's key could grant it `unrestricted` on the strength of the
/// *admin's* role, which that key's own owner could never grant themselves -
/// precisely the "promoting the owner must not widen keys already issued"
/// failure this feature exists to close, from the other direction.
pub(super) fn validate_requested_permissions(
    owner_role: Role,
    requested: &[String],
) -> Result<(), StatusCode> {
    match requested {
        [] => Ok(()),
        [single] if single == Permission::Unrestricted.as_policy() => {
            if owner_role == Role::ServerAdmin {
                Ok(())
            } else {
                Err(StatusCode::BAD_REQUEST)
            }
        }
        entries if entries.iter().all(|e| is_store_scope_entry(e)) => Ok(()),
        _ => Err(StatusCode::BAD_REQUEST),
    }
}

#[cfg(test)]
mod permission_scope_tests {
    use super::*;

    #[test]
    fn an_empty_scope_is_always_accepted() {
        assert!(validate_requested_permissions(Role::User, &[]).is_ok());
        assert!(validate_requested_permissions(Role::ServerAdmin, &[]).is_ok());
    }

    #[test]
    fn unrestricted_is_accepted_only_for_a_server_admin_owner() {
        let unrestricted = vec![Permission::Unrestricted.as_policy().to_string()];
        assert!(validate_requested_permissions(Role::ServerAdmin, &unrestricted).is_ok());
        assert!(validate_requested_permissions(Role::User, &unrestricted).is_err());
    }

    #[test]
    fn a_named_server_permission_is_rejected_even_for_an_admin_owner() {
        // Nothing in this server enforces named server/user permissions
        // individually - accepting one here would promise scoping the rest
        // of the codebase cannot deliver.
        let named = vec![Permission::ServerManageTokens.as_policy().to_string()];
        assert!(validate_requested_permissions(Role::ServerAdmin, &named).is_err());
    }

    #[test]
    fn unrestricted_mixed_with_anything_else_is_rejected() {
        let mixed = vec![
            Permission::Unrestricted.as_policy().to_string(),
            Permission::ServerViewUsers.as_policy().to_string(),
        ];
        assert!(validate_requested_permissions(Role::ServerAdmin, &mixed).is_err());
    }

    #[test]
    fn a_bare_store_permission_is_accepted_for_any_owner_role() {
        // Store permissions ARE enforced individually, so there is nothing
        // to launder through a wider owner role - unlike `unrestricted`.
        let named = vec![Permission::StoreCreateInvoice.as_policy().to_string()];
        assert!(validate_requested_permissions(Role::User, &named).is_ok());
        assert!(validate_requested_permissions(Role::ServerAdmin, &named).is_ok());
    }

    #[test]
    fn a_store_permission_scoped_to_one_store_is_accepted() {
        let scoped = vec![format!(
            "{}:{}",
            Permission::StoreCreateInvoice.as_policy(),
            Uuid::new_v4()
        )];
        assert!(validate_requested_permissions(Role::User, &scoped).is_ok());
    }

    #[test]
    fn several_store_permissions_together_are_accepted() {
        let many = vec![
            Permission::StoreCreateInvoice.as_policy().to_string(),
            format!(
                "{}:{}",
                Permission::StoreViewSettings.as_policy(),
                Uuid::new_v4()
            ),
        ];
        assert!(validate_requested_permissions(Role::User, &many).is_ok());
    }

    #[test]
    fn a_store_permission_with_a_malformed_store_id_is_rejected() {
        let malformed = vec![format!(
            "{}:not-a-uuid",
            Permission::StoreCreateInvoice.as_policy()
        )];
        assert!(validate_requested_permissions(Role::User, &malformed).is_err());
    }

    #[test]
    fn a_store_permission_mixed_with_unrestricted_is_rejected() {
        let mixed = vec![
            Permission::Unrestricted.as_policy().to_string(),
            Permission::StoreCreateInvoice.as_policy().to_string(),
        ];
        assert!(validate_requested_permissions(Role::ServerAdmin, &mixed).is_err());
    }
}
