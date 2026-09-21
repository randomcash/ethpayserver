//! Capability 7: tell an account something, without ever handing a plugin a
//! recipient to name.
//!
//! Before this, a plugin's only way to reach a merchant at all was the
//! checkout itself: refuse an invoice and let the refusal text carry the
//! whole message. A merchant who never opens the page that would have shown
//! them a warning hears nothing until that refusal - which is the same
//! moment a customer's payment fails.
//!
//! A plugin cannot send email. It has no host capability that takes a
//! recipient, and it should not get one - a plugin holding an arbitrary
//! address is a mail relay wearing a billing plugin's name. What this grants
//! instead is narrower: an account id the host already recognises, and a
//! subject and body the host delivers through whichever channel and to
//! whichever address it looks up itself. The plugin never sees either.
//!
//! No new type here for the message content - see [`AccountNotice`] in
//! `crate::services::email`, re-exported alongside the other capabilities'
//! names, the same way capability 1 re-exports `data_service`'s type instead
//! of duplicating it.
//!
//! Right now there is exactly one channel: the address on the account, by
//! email. A wallet-only account has none, and [`notice_address`] says so
//! rather than the notice being silently dropped - a caller that cannot tell
//! "delivered" from "nowhere to deliver to" would have no way to decide
//! whether that is worth surfacing to an operator.
//!
//! Not yet reachable from a plugin: the wasm import this would sit behind is
//! a `payserver-plugin-host` change, pinned by revision in this repo's
//! `Cargo.toml`, and out of scope here. This is the host-side implementation
//! the interface can be written and tested against first - capability 3
//! (`invoice_issuer`) was built the same way before its wasmtime wiring
//! landed.

use async_trait::async_trait;
use auth::{SessionService, UserRepository};
use types::UserId;

use crate::services::email::AccountNotice;
use crate::state::PgAppState;

/// Tell one account something, through a channel and an address the caller
/// never names.
#[async_trait]
pub trait AccountNotifier: Send + Sync {
    /// # Errors
    /// A message describing why the account could not be reached - it does
    /// not exist, has no address on any channel this host can use, or
    /// delivery itself failed.
    async fn notify_account(
        &self,
        account_id: UserId,
        notice: &AccountNotice,
    ) -> Result<(), String>;
}

/// The capability, over the instance's own user directory and email sender.
pub struct PluginAccountNotifier<A> {
    state: PgAppState<A>,
}

impl<A> PluginAccountNotifier<A> {
    #[must_use]
    pub fn new(state: PgAppState<A>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl<A: SessionService + 'static> AccountNotifier for PluginAccountNotifier<A> {
    async fn notify_account(
        &self,
        account_id: UserId,
        notice: &AccountNotice,
    ) -> Result<(), String> {
        let user = UserRepository::get_user(&*self.state.data_service, account_id)
            .await
            .map_err(|e| format!("could not read this account: {e}"))?
            .ok_or_else(|| format!("{account_id} is not an account on this instance"))?;

        let email = notice_address(user.email)?;

        self.state
            .email_sender
            .send_account_notice(&email, notice)
            .await
            .map_err(|e| e.to_string())
    }
}

/// The address this capability would deliver a notice to, or why it cannot.
///
/// Pulled out as a pure function so the one property that matters here - a
/// whitespace-only email is exactly as absent as no email at all - has a
/// test that needs no database. A wallet-only account (`email: None`) is the
/// ordinary case this refuses, not an edge case.
fn notice_address(email: Option<String>) -> Result<String, String> {
    match email.map(|e| e.trim().to_string()) {
        Some(email) if !email.is_empty() => Ok(email),
        _ => Err(
            "this account has no email on file; there is no channel this host can reach it through"
                .to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_account_with_an_email_is_notifiable() {
        assert_eq!(
            notice_address(Some("merchant@example.com".to_string())),
            Ok("merchant@example.com".to_string())
        );
    }

    /// A wallet-only account is the ordinary case, not an edge case: it has
    /// no email at all, and that must be a named refusal rather than a
    /// notice that silently goes nowhere.
    #[test]
    fn a_wallet_only_account_has_no_channel() {
        let Err(err) = notice_address(None) else {
            panic!("an account with no email must not resolve to an address");
        };
        assert!(err.contains("no email on file"), "{err}");
    }

    /// Whitespace is not an address. Treating it as one would hand the email
    /// transport a string that is not a mailbox, and the failure would come
    /// back from SMTP instead of naming the actual cause.
    #[test]
    fn a_blank_email_is_treated_as_no_email() {
        let Err(err) = notice_address(Some("   ".to_string())) else {
            panic!("a whitespace-only email must not resolve to an address");
        };
        assert!(err.contains("no email on file"), "{err}");
    }
}
