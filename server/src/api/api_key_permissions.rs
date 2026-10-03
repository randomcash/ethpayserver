//! Validating the permission scope requested for an API key.
//!
//! Split out of `users` because the request validation and its unit tests
//! were a sizeable share of that file. The scope a key is created with is
//! checked here; there is deliberately no endpoint for editing an existing
//! key's scope after the fact - see the ticket referenced in the commit that
//! removed it.

use axum::http::StatusCode;
use uuid::Uuid;

use auth::{Permission, Policies};

use super::api_key_scope::STANDING_PUSH_SCOPE;

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
/// The one server policy accepted is `ethpay.server.canviewusers`, which
/// `MerchantReader` enforces: read-only access to the merchant listing, and
/// only for a key whose owner is a `ServerAdmin` at request time. Held by a
/// non-admin's key it grants nothing.
///
/// The standing-push entry is accepted only on its own. A key that can write
/// the standing that gates invoice creation must not be able to do anything
/// else, so it cannot be combined with store permissions or the listing scope.
///
/// This takes no role, because none of its answers depend on one. Every
/// accepted form resolves against whoever owns the key rather than naming a
/// power directly: `[]` and `["unrestricted"]` both mean "the owner's role in
/// full", and a store entry is intersected with `user_has_store_permission`
/// at every call site. Every rejected form is rejected for an admin too.
///
/// `["unrestricted"]` briefly required a `ServerAdmin` owner. That refused a
/// spelling rather than a grant: for a non-admin it produces exactly what
/// `[]` produces - a NULL scope - and `[]` was accepted either way. The two
/// are indistinguishable at every gate, since
/// `key_retains_unrestricted_access` is true for both and neither triggers
/// the scoped-key downgrade, so the condition failed a request while the
/// other name for it succeeded. The ticket asked for `unrestricted` to keep
/// working; it does.
///
/// "A key can never exceed its owner" is enforced at read time, not here -
/// `validate_api_key`'s downgrade, the intersection in
/// `key_grants_store_permission`, and every `role == Role::ServerAdmin`
/// check downstream. That is also what covers a key whose owner's role
/// changes after the key was issued, which no write-time check could.
pub(super) fn validate_requested_permissions(requested: &[String]) -> Result<(), StatusCode> {
    match requested {
        [] => Ok(()),
        [single] if single == Permission::Unrestricted.as_policy() => Ok(()),
        [single] if single == STANDING_PUSH_SCOPE => Ok(()),
        entries
            if entries
                .iter()
                .all(|e| is_store_scope_entry(e) || e == Policies::SERVER_VIEW_USERS) =>
        {
            Ok(())
        }
        _ => Err(StatusCode::BAD_REQUEST),
    }
}

#[cfg(test)]
mod permission_scope_tests {
    use super::*;

    #[test]
    fn an_empty_scope_is_always_accepted() {
        assert!(validate_requested_permissions(&[]).is_ok());
    }

    #[test]
    fn unrestricted_is_accepted_for_any_owner_because_it_resolves_against_their_own_role() {
        let unrestricted = vec![Permission::Unrestricted.as_policy().to_string()];
        // Not an admin-only spelling: for a plain user this resolves to the
        // same NULL scope `[]` does, and `[]` is accepted just above.
        // Refusing it would fail the request while the other name for the
        // identical grant succeeded.
        assert!(validate_requested_permissions(&unrestricted).is_ok());
    }

    #[test]
    fn a_named_server_permission_is_rejected_even_for_an_admin_owner() {
        // Nothing in this server enforces named server/user permissions
        // individually - accepting one here would promise scoping the rest
        // of the codebase cannot deliver.
        let named = vec![Permission::ServerManageTokens.as_policy().to_string()];
        assert!(validate_requested_permissions(&named).is_err());
    }

    #[test]
    fn the_merchant_read_policy_is_accepted_alone_or_beside_store_permissions() {
        let view = Permission::ServerViewUsers.as_policy().to_string();
        assert!(validate_requested_permissions(std::slice::from_ref(&view)).is_ok());
        let with_invoice = vec![view, Permission::StoreCreateInvoice.as_policy().to_string()];
        assert!(validate_requested_permissions(&with_invoice).is_ok());
    }

    #[test]
    fn the_standing_push_scope_is_accepted_only_alone() {
        let push = STANDING_PUSH_SCOPE.to_string();
        assert!(validate_requested_permissions(std::slice::from_ref(&push)).is_ok());
        for other in [
            Permission::StoreCreateInvoice.as_policy(),
            Permission::ServerViewUsers.as_policy(),
            Permission::Unrestricted.as_policy(),
        ] {
            let mixed = vec![push.clone(), other.to_string()];
            assert!(validate_requested_permissions(&mixed).is_err(), "{other}");
        }
    }

    #[test]
    fn the_merchant_read_policy_does_not_unlock_its_write_sibling() {
        let manage = vec![
            Permission::ServerViewUsers.as_policy().to_string(),
            Permission::ServerManageUsers.as_policy().to_string(),
        ];
        assert!(validate_requested_permissions(&manage).is_err());
    }

    #[test]
    fn unrestricted_mixed_with_anything_else_is_rejected() {
        let mixed = vec![
            Permission::Unrestricted.as_policy().to_string(),
            Permission::ServerViewUsers.as_policy().to_string(),
        ];
        assert!(validate_requested_permissions(&mixed).is_err());
    }

    #[test]
    fn a_bare_store_permission_is_accepted_for_any_owner_role() {
        // Store permissions ARE enforced individually, so there is nothing
        // to launder through a wider owner role - unlike `unrestricted`.
        let named = vec![Permission::StoreCreateInvoice.as_policy().to_string()];
        assert!(validate_requested_permissions(&named).is_ok());
    }

    #[test]
    fn a_store_permission_scoped_to_one_store_is_accepted() {
        let scoped = vec![format!(
            "{}:{}",
            Permission::StoreCreateInvoice.as_policy(),
            Uuid::new_v4()
        )];
        assert!(validate_requested_permissions(&scoped).is_ok());
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
        assert!(validate_requested_permissions(&many).is_ok());
    }

    #[test]
    fn a_store_permission_with_a_malformed_store_id_is_rejected() {
        let malformed = vec![format!(
            "{}:not-a-uuid",
            Permission::StoreCreateInvoice.as_policy()
        )];
        assert!(validate_requested_permissions(&malformed).is_err());
    }

    #[test]
    fn a_store_permission_mixed_with_unrestricted_is_rejected() {
        let mixed = vec![
            Permission::Unrestricted.as_policy().to_string(),
            Permission::StoreCreateInvoice.as_policy().to_string(),
        ];
        assert!(validate_requested_permissions(&mixed).is_err());
    }
}
