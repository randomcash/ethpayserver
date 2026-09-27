//! User API endpoints: API keys, wallet login credentials, account deletion
//! and email change.
//!
//! All endpoints require authentication via session token.
//!
//! Split by behaviour rather than kept as one file - each submodule covers
//! one of the groupings above, along the same lines as `admin::deletion`.

pub(crate) mod api_keys;
mod deletion;
mod email;
mod key_material;
pub(crate) mod wallets;

pub use crate::api::api_key_permissions::{
    UpdateApiKeyPermissionsPayload, update_api_key_permissions,
};
pub use api_keys::{create_api_key, list_api_keys, revoke_api_key, rotate_api_key, update_api_key};
pub use deletion::{DeleteAccountQuery, delete_account};
pub use email::{
    ConfirmEmailChangePayload, RequestEmailChangePayload, confirm_email_change, remove_email,
    request_email_change,
};
pub use wallets::{
    PromoteWalletCredentialRequest, WalletCredentialResponse, WalletReauthChallengeResponse,
    create_wallet_reauth_challenge, list_wallet_credentials, set_primary_wallet_credential,
};

// `ApiKeyInfoResponse`/`CreateApiKeyPayload`/`CreateApiKeyResponsePayload`/
// `RotateApiKeyResponsePayload` come from `crate::api::api_key_types`
// (hand-mirrored from the pinned `api-types` crate, plus a `permissions`
// field that crate does not have yet - see that module's doc comment) rather
// than from `api_types` directly. `ApiKeyListResponse` follows since it
// wraps `ApiKeyInfoResponse`. `UpdateApiKeyPayload` carries no permissions
// and is unaffected, so it still comes straight from `api_types`.
pub use crate::api::api_key_types::{
    ApiKeyInfoResponse, ApiKeyListResponse, CreateApiKeyPayload, CreateApiKeyResponsePayload,
    RotateApiKeyResponsePayload,
};
pub use api_types::UpdateApiKeyPayload;
