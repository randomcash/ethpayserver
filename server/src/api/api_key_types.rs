//! API key wire types, hand-mirrored from the pinned `api-types` crate.
//!
//! Split out of `users` - these DTOs plus the functions that build them were
//! most of that file's growth when `permissions` needed to reach the wire.
//!
//! `api-types` lives in payserver-commons, and landing a field there is the
//! three-step dance (merge, bump the pinned rev, `cargo update`) this repo's
//! `CLAUDE.md` describes - a cross-repo change a single-ticket pass here
//! cannot complete on its own. Hand-mirroring only grows the wire shape by a
//! field, so a client still built against the pinned type keeps working
//! unchanged.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use auth::{ApiKey, ApiKeyInfo};
use data_service::ApiKeyFullInfo;

/// API key info for list/get responses. See the module doc for why this is
/// hand-mirrored rather than extending the pinned `api_types` struct.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct ApiKeyInfoResponse {
    pub id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    /// Per-key rate limit in requests per minute. Null = server default.
    pub rate_limit_rpm: Option<i32>,
    /// Set when the key is deprecated via rotation. Key remains valid during
    /// the grace window; null means not deprecated.
    pub deprecated_at: Option<DateTime<Utc>>,
    /// When the grace window ends for a deprecated key. Null for
    /// non-deprecated keys.
    pub deprecation_expires_at: Option<DateTime<Utc>>,
    /// Permission policy strings this key is scoped to. `None` means it
    /// inherits its owner's role in full - either because it predates this
    /// column, or because nobody has narrowed it since.
    pub permissions: Option<Vec<String>>,
}

/// Response for listing API keys.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct ApiKeyListResponse {
    pub keys: Vec<ApiKeyInfoResponse>,
}

/// Request to create a new API key. See `ApiKeyInfoResponse` for why this is
/// hand-mirrored rather than extending the pinned `api_types` struct.
#[derive(Debug, Clone, serde::Deserialize, utoipa::ToSchema)]
pub struct CreateApiKeyPayload {
    /// Human-readable name for the key.
    pub name: String,
    /// Optional expiration time.
    pub expires_at: Option<DateTime<Utc>>,
    /// The key's scope, chosen at creation. Defaults to `[]` when omitted: a
    /// new key starts able to do nothing beyond authenticating and must be
    /// deliberately widened, rather than silently inheriting everything its
    /// owner can do.
    ///
    /// `["unrestricted"]` (see `auth::Policies::UNRESTRICTED`) inherits the
    /// owner's role in full, and is only accepted when the caller's own
    /// current role grants it - a key can never exceed its owner, including
    /// at the moment it is minted.
    ///
    /// Otherwise, any combination of `ethpay.store.*` policy strings (e.g.
    /// `"ethpay.store.cancreateinvoice"`), each optionally suffixed with
    /// `:<storeId>` to narrow the grant to one store rather than every
    /// store the owner can reach. These are real, individually-enforced
    /// grants - `user_has_store_permission` already checks each one in SQL -
    /// unlike `ethpay.server.*`/`ethpay.user.*` policies, which nothing in
    /// this server gates on individually and which `validate_requested_permissions`
    /// therefore still refuses to name one at a time.
    #[serde(default)]
    pub permissions: Vec<String>,
}

/// Response after creating an API key (includes plaintext key).
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct CreateApiKeyResponsePayload {
    pub id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    /// The plaintext API key. Store this securely — it cannot be retrieved again.
    pub key: String,
    pub permissions: Vec<String>,
}

/// Response after rotating an API key.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct RotateApiKeyResponsePayload {
    /// The new API key's ID.
    pub id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub created_at: DateTime<Utc>,
    /// The new plaintext API key. Store this securely.
    pub key: String,
    /// When the old key was deprecated (grace window starts here).
    pub old_key_deprecated_at: DateTime<Utc>,
    /// When the old key's grace window ends and it stops authenticating.
    /// Clients should show this directly instead of hardcoding "48 hours".
    pub old_key_grace_expires_at: DateTime<Utc>,
    /// Permission scope carried over from the key being rotated.
    pub permissions: Option<Vec<String>>,
}

/// Translate a `deprecated_at` into the grace-window deadline. Uses the same
/// grace-seconds value as the auth extractor, so the client-visible expiry
/// matches when the server actually starts rejecting the key.
pub(crate) fn deprecation_expires_at(deprecated_at: DateTime<Utc>) -> DateTime<Utc> {
    deprecated_at + chrono::Duration::seconds(super::extractors::deprecation_grace_secs())
}

/// Build from an `ApiKey` plus the ancillary rate-limit / deprecation /
/// permission fields not present on the auth-crate struct. Used by
/// endpoints that already have an `ApiKey` in hand (e.g. update_api_key
/// after a mutation).
pub(crate) fn api_key_info_with_rate_limit(
    key: &ApiKey,
    rate_limit_rpm: Option<i32>,
    deprecated_at: Option<DateTime<Utc>>,
    permissions: Option<Vec<String>>,
) -> ApiKeyInfoResponse {
    let info = ApiKeyInfo::from(key);
    ApiKeyInfoResponse {
        id: info.id.0,
        name: info.name,
        key_prefix: info.key_prefix,
        is_active: info.is_active,
        created_at: info.created_at,
        last_used_at: info.last_used_at,
        expires_at: info.expires_at,
        rate_limit_rpm,
        deprecated_at,
        deprecation_expires_at: deprecated_at.map(deprecation_expires_at),
        permissions,
    }
}

/// Build the wire shape from the `auth` domain type.
///
/// A free function rather than a `From` impl: `ApiKeyFullInfo` belongs to
/// `data_service` and `ApiKeyInfoResponse` is local to this module (see its
/// doc comment), so neither owns the other.
pub(crate) fn api_key_info_response(info: ApiKeyFullInfo) -> ApiKeyInfoResponse {
    ApiKeyInfoResponse {
        id: info.id,
        name: info.name,
        key_prefix: info.key_prefix,
        is_active: info.is_active,
        created_at: info.created_at,
        last_used_at: info.last_used_at,
        expires_at: info.expires_at,
        rate_limit_rpm: info.rate_limit_rpm,
        deprecated_at: info.deprecated_at,
        deprecation_expires_at: info.deprecated_at.map(deprecation_expires_at),
        permissions: info.permissions,
    }
}
