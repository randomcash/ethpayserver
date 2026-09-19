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
use payserver_plugin_host::{PageRenderError, PageRenderer, PluginHost};
use serde::Serialize;

/// The export asked to draw a page.
pub const RENDER_PAGE: &str = "render_page";

#[derive(Debug, Serialize)]
struct WirePageRequest<'a> {
    path: &'a str,
    viewer: Viewer,
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
        path: &str,
        viewer: Viewer,
    ) -> Result<Option<PageElement>, PageRenderError> {
        let request = WirePageRequest { path, viewer };

        self.host
            .run_query::<_, Option<PageElement>>(&self.plugin, RENDER_PAGE, &request)
            .await
            .map_err(|reason| {
                // Logged with the plugin id because the message the caller
                // gets is deliberately about the page, not about wasm.
                tracing::warn!(
                    plugin_id = %self.plugin,
                    path,
                    reason = %reason,
                    "a plugin could not render one of its pages"
                );
                PageRenderError::new(reason)
            })
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
        })
        .unwrap();
        assert_eq!(json, r#"{"path":"subscriptions","viewer":"merchant"}"#);

        let admin = serde_json::to_string(&WirePageRequest {
            path: "subscriptions",
            viewer: Viewer::Admin,
        })
        .unwrap();
        assert!(admin.contains(r#""viewer":"admin""#), "{admin}");
    }

    /// `null` is how a plugin says "not one of my pages", and it has to
    /// survive as `None` rather than failing to parse - otherwise an unknown
    /// path reports a broken plugin instead of a missing page.
    #[test]
    fn a_null_answer_is_a_missing_page_not_a_parse_failure() {
        let parsed: Option<PageElement> = serde_json::from_str("null").unwrap();
        assert!(parsed.is_none());
    }

    /// An answer this client does not recognise must still render. The
    /// vocabulary grows, and a plugin built against a newer host would
    /// otherwise produce a page that fails to parse here and 500s.
    #[test]
    fn an_unrecognised_element_parses_as_unknown_rather_than_failing() {
        let parsed: Option<PageElement> =
            serde_json::from_str(r#"{"type":"hologram","glow":true}"#).unwrap();
        assert_eq!(parsed, Some(PageElement::Unknown));
    }
}
