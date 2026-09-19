//! Capability 5: a plugin draws its own page.
//!
//! A plugin cannot ship Rust into an already-compiled Leptos client, so it
//! ships a [`PageElement`] tree and the client draws it. `PageHost` is the
//! registry of who answers for which plugin id; this is the implementation
//! that answers by actually running the plugin.
//!
//! Until this existed, [`PageRenderer`] had no production implementation at
//! all. `PageHost` was constructed empty and every page request 404'd -
//! correct behaviour for a host with nothing to render, and indistinguishable
//! from a feature that was never wired up. It was the latter.
//!
//! # What the plugin is asked, and what it may answer
//!
//! ```json
//! {"path": "subscriptions", "viewer": "merchant"}
//! ```
//!
//! and it answers a `PageElement`, or `null` for a path it does not serve.
//! The viewer is the host's to decide, never the plugin's to claim: it comes
//! from the authenticated session in `api::plugins`, and this layer only
//! passes it along. A plugin that could name its own viewer could ask to be
//! treated as an admin.
//!
//! # Why a failure is not an empty page
//!
//! `run_query`, not `run_filter`. A filter that cannot run falls back to its
//! manifest's failure mode because it still owes a verdict; a page has no
//! such default. Answering an empty page for a plugin that trapped would put
//! a blank panel where a merchant expects to see what they owe, which is the
//! single worst thing this layer can produce - so the failure travels and the
//! client shows that instead.

use std::sync::Arc;

use async_trait::async_trait;
use payserver_plugin_api::PluginId;
use payserver_plugin_api::page::{PageElement, Viewer};
use payserver_plugin_host::{PageRenderError, PageRenderer, PageRequest, PluginHost};
use serde::{Deserialize, Serialize};

/// The export asked to draw a page.
pub const RENDER_PAGE: &str = "render_page";

#[derive(Debug, Serialize)]
struct WirePageRequest<'a> {
    path: &'a str,
    viewer: Viewer,
    /// Who is asking, as the host resolved them. A plugin needs this to
    /// show a merchant their own anything, and must never be able to supply
    /// it - one that could name an account could read another merchant's
    /// bill.
    #[serde(skip_serializing_if = "Option::is_none")]
    account_id: Option<&'a str>,
}

/// What a plugin answers a page request with.
///
/// A failure travels **in the answer**, not in the return value. The ABI packs
/// `(ptr << 32) | len` into the i64 an export returns and has no
/// negative-means-error convention there - that belongs to the host-call
/// direction. A plugin that returned -1 to mean "I failed" had it read as
/// `ptr = len = 0xFFFFFFFF`, which the runtime reports as a malformed answer
/// and counts against the failure budget: three page loads disabled the
/// plugin outright. A plugin that cannot draw one page must not be able to
/// switch itself off by saying so.
///
/// Untagged, and the order matters: `{"error": ...}` is tried first, so a
/// page can never be mistaken for a failure, and `null` and a real element
/// both fall through to `Page`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum WirePageAnswer {
    Failed { error: String },
    Page(Option<PageElement>),
}

/// Renders one plugin's pages by calling its `render_page` export.
pub struct WasmPageRenderer {
    host: Arc<PluginHost>,
    plugin: PluginId,
}

impl WasmPageRenderer {
    #[must_use]
    pub fn new(host: Arc<PluginHost>, plugin: PluginId) -> Self {
        Self { host, plugin }
    }
}

#[async_trait]
impl PageRenderer for WasmPageRenderer {
    async fn render_page(
        &self,
        request: &PageRequest,
    ) -> Result<Option<PageElement>, PageRenderError> {
        let wire = WirePageRequest {
            path: &request.path,
            viewer: request.viewer,
            account_id: request.account_id.as_deref(),
        };

        let answer = self
            .host
            .run_query::<_, WirePageAnswer>(&self.plugin, RENDER_PAGE, &wire)
            .await
            .map_err(|reason| self.refused(&request.path, reason))?;

        match answer {
            WirePageAnswer::Page(page) => Ok(page),
            WirePageAnswer::Failed { error } => Err(self.refused(&request.path, error)),
        }
    }
}

impl WasmPageRenderer {
    /// One place to log a refusal, so the two ways a page can fail - the call
    /// not completing, and the plugin saying it could not - read the same in
    /// the log and to the caller.
    fn refused(&self, path: &str, reason: String) -> PageRenderError {
        // Logged with the plugin id because the message the caller gets is
        // deliberately about the page, not about wasm.
        tracing::warn!(
            plugin_id = %self.plugin,
            path,
            reason = %reason,
            "a plugin could not render one of its pages"
        );
        PageRenderError::new(reason)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// The viewer must cross as the host's own vocabulary. A plugin matching
    /// on `"merchant"` and receiving `"Merchant"` would fall through to its
    /// default branch, which for a billing page is the wrong audience.
    #[test]
    fn the_request_names_the_viewer_the_way_the_page_vocabulary_does() {
        let json = serde_json::to_string(&WirePageRequest {
            path: "subscriptions",
            viewer: Viewer::Merchant,
            account_id: Some("acct-7"),
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"path":"subscriptions","viewer":"merchant","account_id":"acct-7"}"#
        );

        let admin = serde_json::to_string(&WirePageRequest {
            path: "subscriptions",
            viewer: Viewer::Admin,
            account_id: None,
        })
        .unwrap();
        assert!(admin.contains(r#""viewer":"admin""#), "{admin}");
        assert!(
            !admin.contains("account_id"),
            "an absent identity must be absent, not null: {admin}"
        );
    }

    /// `null` is how a plugin says "not one of my pages", and it has to
    /// survive as `None` rather than failing to parse - otherwise an unknown
    /// path reports a broken plugin instead of a missing page.
    #[test]
    fn a_null_answer_is_a_missing_page_not_a_parse_failure() {
        let parsed: WirePageAnswer = serde_json::from_str("null").unwrap();
        assert!(matches!(parsed, WirePageAnswer::Page(None)));
    }

    /// The path that shipped untested and cost a live outage. A plugin
    /// reports a failure in its answer; returning a negative from the export
    /// instead had the runtime read it as `ptr = len = 0xFFFFFFFF`, call the
    /// answer malformed, and disable the plugin after three page loads.
    #[test]
    fn a_plugin_can_report_a_failure_without_looking_broken() {
        let parsed: WirePageAnswer =
            serde_json::from_str(r#"{"error":"the database refused the query"}"#).unwrap();
        let WirePageAnswer::Failed { error } = parsed else {
            panic!("an error answer must not parse as a page");
        };
        assert_eq!(error, "the database refused the query");
    }

    /// The untagged order has to keep a page out of the failure arm. A page
    /// that parsed as a failure would take the plugin down for rendering
    /// correctly.
    #[test]
    fn a_real_page_never_parses_as_a_failure() {
        let parsed: WirePageAnswer =
            serde_json::from_str(r#"{"type":"section","title":"Your subscription"}"#).unwrap();
        assert!(
            matches!(parsed, WirePageAnswer::Page(Some(PageElement::Section(_)))),
            "a section must parse as a page"
        );
    }

    /// An answer this client does not recognise must still render. The
    /// vocabulary grows, and a plugin built against a newer host would
    /// otherwise produce a page that fails to parse here and 500s.
    #[test]
    fn an_unrecognised_element_parses_as_unknown_rather_than_failing() {
        let parsed: WirePageAnswer =
            serde_json::from_str(r#"{"type":"hologram","glow":true}"#).unwrap();
        assert!(matches!(
            parsed,
            WirePageAnswer::Page(Some(PageElement::Unknown))
        ));
    }
}
