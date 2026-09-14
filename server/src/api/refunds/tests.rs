//! Refunds are the merchant's job, not this server's (RCS-272).
//!
//! `create_refund` used to check an amount and write a `Pending` row that
//! nothing downstream would ever move — a promise the server could not keep,
//! since it holds no spending key. These tests pin the replacement: every
//! call gets the same explicit refusal, not a row that looks like progress.

use super::{REFUND_UNSUPPORTED_REASON, refund_unsupported};

// `ApiErr` is a private tuple struct defined in `crate::api`; this module is
// a descendant of it, so its fields are visible here even though
// `refunds.rs` itself only ever sees `ApiErr` as an opaque `IntoResponse`.
use crate::api::ApiErr;

use axum::http::StatusCode;

#[test]
fn a_refund_request_is_refused_not_queued() {
    let ApiErr(status, reason) = refund_unsupported();

    assert_eq!(
        status,
        StatusCode::NOT_IMPLEMENTED,
        "a refund the server can never carry out must not look like an accepted one"
    );
    assert_eq!(reason, REFUND_UNSUPPORTED_REASON);
}

#[test]
fn the_refusal_names_the_reason() {
    // A bare status with no body reaches a caller as "HTTP error 501:" and
    // nothing else — the same dead end `ApiErr`'s own doc comment describes
    // for a reasonless 409. A merchant hitting this endpoint needs to learn
    // to refund from their own wallet, not guess why the request failed.
    assert!(!REFUND_UNSUPPORTED_REASON.is_empty());
}
