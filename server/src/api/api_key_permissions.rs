//! Narrowing an existing API key's permission scope below its owner's role.
//!
//! Split out of `users` - request validation, the write guards, and the
//! `PATCH /users/api-keys/{id}/permissions` handler itself, together with
//! their unit tests, were about a third of that file's line count.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use uuid::Uuid;

use auth::{ApiKeyId, ApiKeyRepository, Permission, Policies, Role, SessionService, UserId};

use super::extractors::AuthenticatedUser;
use super::users::{ApiKeyInfoResponse, api_key_info_with_rate_limit};
use crate::state::PgAppState;

/// Body for `PATCH /users/api-keys/{id}/permissions`.
#[derive(Debug, Clone, serde::Deserialize, utoipa::ToSchema)]
pub struct UpdateApiKeyPermissionsPayload {
    /// `None`/`null` clears the key back to "inherit the owner's role in
    /// full". `Some` sets an explicit scope - see `CreateApiKeyPayload`'s
    /// `permissions` field for what is accepted and why.
    pub permissions: Option<Vec<String>>,
}

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

/// Whether `caller` may manage a given API key at all: its own owner always
/// may; anyone else needs `ServerAdmin`. Used by every endpoint in this
/// module that mutates or reveals a specific key by id.
fn caller_may_manage_key(caller_id: UserId, caller_role: Role, key_owner_id: UserId) -> bool {
    caller_id == key_owner_id || caller_role == Role::ServerAdmin
}

/// The role a requested permission scope must be validated against: always
/// the key's own current owner, resolved without trusting the caller's role
/// when the caller isn't that owner. This is the exact mechanism that stops
/// a `ServerAdmin` editing someone else's key from laundering a grant
/// through their own broader role - see `update_api_key_permissions`'s call
/// site, which only fetches `fetched_owner_role` from the database when
/// `caller_id != key_owner_id`.
fn owner_role_for_permission_check(
    caller_id: UserId,
    caller_role: Role,
    key_owner_id: UserId,
    fetched_owner_role: Role,
) -> Role {
    if caller_id == key_owner_id {
        caller_role
    } else {
        fetched_owner_role
    }
}

/// Whether `caller_role` may reset a key's permissions back to `None`
/// ("inherit the owner's role in full"). Only a `ServerAdmin` may - a key
/// that has itself been narrowed away from `unrestricted` authenticates as
/// a plain `User` (`validate_api_key`'s downgrade), so without this it
/// could use this endpoint on its own row to undo its own narrowing.
fn may_clear_to_inherit(caller_role: Role) -> bool {
    caller_role == Role::ServerAdmin
}

#[cfg(test)]
mod update_permissions_guard_tests {
    use super::*;

    #[test]
    fn a_key_owner_may_always_manage_their_own_key() {
        let uid = UserId(Uuid::new_v4());
        assert!(caller_may_manage_key(uid, Role::User, uid));
    }

    #[test]
    fn a_non_owner_needs_server_admin_to_manage_someone_elses_key() {
        let caller = UserId(Uuid::new_v4());
        let owner = UserId(Uuid::new_v4());
        assert!(!caller_may_manage_key(caller, Role::User, owner));
        assert!(caller_may_manage_key(caller, Role::ServerAdmin, owner));
    }

    #[test]
    fn owner_role_check_uses_the_callers_own_role_when_they_own_the_key() {
        let uid = UserId(Uuid::new_v4());
        // fetched_owner_role is irrelevant here - the real handler never even
        // fetches it in this branch - so pass a deliberately wrong value to
        // prove it is ignored.
        assert_eq!(
            owner_role_for_permission_check(uid, Role::ServerAdmin, uid, Role::User),
            Role::ServerAdmin
        );
    }

    #[test]
    fn owner_role_check_ignores_the_caller_when_editing_someone_elses_key() {
        // The scenario the doc comment exists for: a ServerAdmin editing a
        // plain User's key must be validated against that User's role, not
        // the admin's own - otherwise the admin could launder an
        // `unrestricted` grant onto a key whose owner could never hold it.
        let admin = UserId(Uuid::new_v4());
        let target_user = UserId(Uuid::new_v4());
        assert_eq!(
            owner_role_for_permission_check(admin, Role::ServerAdmin, target_user, Role::User),
            Role::User
        );
    }

    #[test]
    fn only_a_server_admin_caller_may_clear_a_key_back_to_inherit() {
        assert!(may_clear_to_inherit(Role::ServerAdmin));
        assert!(!may_clear_to_inherit(Role::User));
    }
}

/// Narrow (or, for a still-fully-privileged caller, widen back to
/// "inherit") an existing API key's permission scope.
///
/// A separate endpoint from `update_api_key` rather than folded into it:
/// that endpoint always overwrites every field in its body, and a caller
/// that only wants to change the rate limit must not be able to
/// accidentally reset a deliberately narrowed key back to full access just
/// by omitting a field it does not know exists.
///
/// Clearing to "inherit" (`permissions: null`) is refused unless the caller
/// is unrestricted for *this* request - not merely `role == ServerAdmin` on
/// the account, but actually holding that role right now. A key that has
/// itself been scoped away from `unrestricted` authenticates as a plain
/// `User` (see `validate_api_key`), so it cannot use this endpoint to hand
/// itself back the access it was narrowed away from, even against its own
/// row.
#[utoipa::path(
    patch,
    path = "/users/api-keys/{id}/permissions",
    tag = "users",
    security(("bearer_auth" = [])),
    params(
        ("id" = Uuid, Path, description = "API key ID to update"),
    ),
    request_body = UpdateApiKeyPermissionsPayload,
    responses(
        (status = 200, description = "Permission scope updated", body = ApiKeyInfoResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "API key not found"),
    )
)]
pub async fn update_api_key_permissions<A>(
    AuthenticatedUser(user): AuthenticatedUser,
    State(state): State<PgAppState<A>>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateApiKeyPermissionsPayload>,
) -> Result<Json<ApiKeyInfoResponse>, StatusCode>
where
    A: SessionService + 'static,
{
    let key = state
        .data_service
        .get_api_key(ApiKeyId(id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if !caller_may_manage_key(user.id, user.role, key.user_id) {
        return Err(StatusCode::NOT_FOUND);
    }

    // The role to validate a widened grant against is the key's own owner's,
    // not the caller's - an admin editing someone else's key must not be
    // able to launder a grant through their own broader role. Only fetched
    // when the caller isn't the owner: the common case (a user managing
    // their own key) already has this in hand.
    let owner_role = if key.user_id == user.id {
        owner_role_for_permission_check(user.id, user.role, key.user_id, user.role)
    } else {
        let fetched_owner_role = auth::UserRepository::get_user(&*state.data_service, key.user_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?
            .role;
        owner_role_for_permission_check(user.id, user.role, key.user_id, fetched_owner_role)
    };

    let permissions = match &payload.permissions {
        Some(requested) => {
            validate_requested_permissions(owner_role, requested)?;
            Some(requested.clone())
        }
        None => {
            if !may_clear_to_inherit(user.role) {
                return Err(StatusCode::BAD_REQUEST);
            }
            None
        }
    };

    state
        .data_service
        .update_api_key_permissions(ApiKeyId(id), permissions.as_deref())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let auth_info = state
        .data_service
        .get_api_key_auth_info_by_id(id)
        .await
        .ok()
        .flatten();
    let (rate_limit_rpm, deprecated_at) = auth_info
        .map(|info| (info.rate_limit_rpm, info.deprecated_at))
        .unwrap_or_default();

    Ok(Json(api_key_info_with_rate_limit(
        &key,
        rate_limit_rpm,
        deprecated_at,
        permissions,
    )))
}
