//! Receives an account's standing from the external service that decides it.
//!
//! Core, not a plugin: the standing gates invoice creation, so it is stored by
//! the host, and a push lands whether or not any plugin is loaded. The apply is
//! a compare-and-set on `version` in the data layer; this module only
//! authenticates, validates and reports which of the defined answers applies.
//!
//! Authentication runs before the body is read, so an unauthenticated caller
//! learns nothing about validation.

use auth::SessionService;
use axum::{
    Json,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    routing::MethodRouter,
};
use chrono::{DateTime, Utc};
use data_service::{AccountStanding, AccountStandingStore, ApplyOutcome};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::extractors::StandingPusher;
use crate::state::PgAppState;

/// Largest body accepted. The real payload is a few hundred bytes.
pub const MAX_BODY_BYTES: usize = 4 * 1024;
const MAX_PLAN_NAME_BYTES: usize = 200;
const MAX_CHECKOUT_URL_BYTES: usize = 2048;

#[derive(Debug, Deserialize, ToSchema)]
pub struct PushEntitlementRequest {
    pub account_id: Uuid,
    /// Per account, only rises. A push whose version is not higher than the
    /// held one is acknowledged and changes nothing.
    pub version: i64,
    pub in_good_standing: bool,
    pub paid_through: Option<DateTime<Utc>>,
    pub plan_name: String,
    /// Rendered to merchants as a link, so only `https` is accepted.
    pub checkout_url: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PushEntitlementResponse {
    /// True if this push replaced the held standing.
    pub applied: bool,
    /// The version held after the push.
    pub version: i64,
}

fn validate(req: &PushEntitlementRequest) -> Result<(), &'static str> {
    if req.version < 1 {
        return Err("version must be at least 1");
    }
    if req.plan_name.trim().is_empty() || req.plan_name.len() > MAX_PLAN_NAME_BYTES {
        return Err("plan_name must be non-empty and at most 200 bytes");
    }
    if let Some(url) = &req.checkout_url
        && (url.len() > MAX_CHECKOUT_URL_BYTES
            || !url.starts_with("https://")
            || url.len() == "https://".len()
            || url.chars().any(|c| c.is_whitespace() || c.is_control()))
    {
        return Err("checkout_url must be an https URL of at most 2048 bytes");
    }
    Ok(())
}

/// Receive an account's standing.
///
/// Answers 200 both when the push is applied and when an equal or higher
/// version is already held: a stale push can never apply, so asking the sender
/// to retry it would only raise a false alarm.
#[utoipa::path(
    post,
    path = "/plugins/entitlements",
    tag = "plugins",
    security(("bearer_auth" = [])),
    request_body = PushEntitlementRequest,
    responses(
        (status = 200, description = "Applied, or an equal or higher version is already held", body = PushEntitlementResponse),
        (status = 400, description = "Malformed body or field"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "The credential is not a key scoped to push account standing"),
        (status = 413, description = "Body too large"),
        (status = 503, description = "Database unavailable; retry"),
    )
)]
pub async fn push_entitlement<A>(
    StandingPusher(_pusher): StandingPusher,
    State(state): State<PgAppState<A>>,
    body: Bytes,
) -> Result<Json<PushEntitlementResponse>, (StatusCode, &'static str)>
where
    A: SessionService + 'static,
{
    let req: PushEntitlementRequest = serde_json::from_slice(&body)
        .map_err(|_| (StatusCode::BAD_REQUEST, "malformed entitlement body"))?;
    validate(&req).map_err(|m| (StatusCode::BAD_REQUEST, m))?;

    let pushed = AccountStanding {
        account_id: req.account_id,
        version: req.version,
        in_good_standing: req.in_good_standing,
        paid_through: req.paid_through,
        plan_name: req.plan_name,
        checkout_url: req.checkout_url,
    };
    let unavailable = |e: types::RepositoryError| {
        tracing::error!(error = %e, "account standing store unavailable");
        (StatusCode::SERVICE_UNAVAILABLE, "temporarily unavailable")
    };

    let outcome = state
        .data_service
        .apply_account_standing(&pushed)
        .await
        .map_err(unavailable)?;

    match outcome {
        ApplyOutcome::Applied { version } => Ok(Json(PushEntitlementResponse {
            applied: true,
            version,
        })),
        ApplyOutcome::Kept { held_version } => {
            if held_version == pushed.version
                && let Ok(Some(held)) = state
                    .data_service
                    .get_account_standing(pushed.account_id)
                    .await
                && held.standing != pushed
            {
                // The sender raises the version only when the content changes,
                // so this is its fault. The held value is kept.
                tracing::warn!(
                    account_id = %pushed.account_id,
                    version = pushed.version,
                    "standing push repeated a held version with different content; kept the held value"
                );
            }
            Ok(Json(PushEntitlementResponse {
                applied: false,
                version: held_version,
            }))
        }
    }
}

/// The route's method router, with the body bound the design requires.
pub fn route<A>() -> MethodRouter<PgAppState<A>>
where
    A: SessionService + 'static,
{
    axum::routing::post(push_entitlement::<A>).layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(version: i64, plan: &str, url: Option<&str>) -> PushEntitlementRequest {
        PushEntitlementRequest {
            account_id: Uuid::nil(),
            version,
            in_good_standing: true,
            paid_through: None,
            plan_name: plan.into(),
            checkout_url: url.map(str::to_string),
        }
    }

    #[test]
    fn a_well_formed_push_validates() {
        assert!(validate(&req(1, "p", Some("https://pay.example/c"))).is_ok());
        assert!(validate(&req(1, "p", None)).is_ok());
    }

    #[test]
    fn each_bad_field_is_refused() {
        assert!(validate(&req(0, "p", None)).is_err());
        assert!(validate(&req(-3, "p", None)).is_err());
        assert!(validate(&req(1, "", None)).is_err());
        assert!(validate(&req(1, "   ", None)).is_err());
        assert!(validate(&req(1, &"x".repeat(201), None)).is_err());
        assert!(validate(&req(1, "p", Some("javascript:alert(1)"))).is_err());
        assert!(validate(&req(1, "p", Some("http://insecure.example"))).is_err());
        assert!(validate(&req(1, "p", Some("https://"))).is_err());
        assert!(validate(&req(1, "p", Some("https://a.example/ b"))).is_err());
        let long = format!("https://a.example/{}", "x".repeat(2048));
        assert!(validate(&req(1, "p", Some(&long))).is_err());
    }
}
