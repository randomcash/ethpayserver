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

pub use api_types::{
    ApiKeyInfoResponse, ApiKeyListResponse, CreateApiKeyPayload, CreateApiKeyResponsePayload,
    RotateApiKeyResponsePayload, UpdateApiKeyPayload,
};
