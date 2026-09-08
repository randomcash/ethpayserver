//! The API types the client uses.
//!
//! Almost all of these now come from `api-types` in payserver-commons, which is
//! the same crate the server builds its responses from. That is the point: the
//! client used to keep its own hand-written copy of each shape, the two were
//! compiled separately, and nothing checked that they agreed. When they drifted
//! the whole workspace still built and the page died on contact with the API -
//! three times in one day (RCS-234, and twice in RCS-241).
//!
//! What stays here is what the server does not define: view models the client
//! assembles for itself, and the auth shapes that come from `ui-kit`.
//!
//! The aliases exist so pages keep reading `Invoice` rather than
//! `InvoiceResponse`. The shape is the server's; only the local name is shorter.

pub use api_types::{
    AddMemberRequest, AdminUserInfo, ApiKeyInfoResponse as ApiKeyInfo, ApiKeyListResponse,
    AssetVolume, ChainHealthInfo, ChainsHealthResponse, CheckoutPaymentInfo, CheckoutResponse,
    ConfigureWebhookRequest as UpdateWebhookRequest, CreateApiKeyPayload as CreateApiKeyRequest,
    CreateApiKeyResponsePayload, CreateInvoiceRequest, CreatePaymentMethodRequest,
    CreateStoreRequest, DailyVolume, DashboardAnalytics, DashboardStats, DeepHealthResponse,
    DependencyHealth, DerivedAddressEntry, HealthResponse, InvoiceListResponse,
    InvoiceResponse as Invoice, InvoiceStatusResponse, MemberResponse, MonitorHealth,
    PaginatedResponse, PaymentListResponse, PaymentMethodResponse as StorePaymentMethod,
    PaymentOptionResponse as PaymentOption, PaymentResponse as Payment, PayoutListResponse,
    PayoutResponse, ReadinessResponse, RefundResponse,
    RotateApiKeyResponsePayload as RotateApiKeyResponse, RotateWalletRequest, RotateWalletResponse,
    RotationEntry, RpcHealth, ServerSettingsResponse, SetStoreWalletRequest, SetTokenPolicyRequest,
    StoreResponse as Store, StoreSettingsResponse as StoreSettings, StoreWalletResponse,
    TokenPolicyEntryPayload as TokenPolicyEntry, TokenPolicyResponse as TokenPolicy,
    TxHashLookupResponse, UpdateMemberRequest, UpdatePaymentMethodRequest,
    UpdateRoleRequest as UpdateUserRoleRequest, UpdateServerSettingsRequest, UpdateStoreRequest,
    UpdateStoreSettingsRequest, UpdateWalletRequest, UserListResponse, WalletAddressesResponse,
    WalletResponse as Wallet, WalletXpubResponse, WebhookResponse as StoreWebhook, mask_xpub,
};

// Domain enums the contract references.
pub use types::{ChainId, InvoiceStatus};

mod local;
mod user;

pub use local::*;
pub use user::*;

#[cfg(test)]
mod tests;
