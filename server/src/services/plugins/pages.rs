//! Host-side page resolution for `GET /plugins/{id}/pages/{path}`.
//!
//! No wasmtime runtime has landed on `main` yet (the runtime and the route
//! mounting that would back a real plugin exist only on other, unmerged
//! branches at the time of writing), so there is no plugin code this host
//! can actually invoke. [`PageRenderer`] is the seam a real plugin will
//! implement once that runtime exists; production wiring starts with an
//! empty [`PageHost`], so every request 404s until a renderer is registered.
//! That is correct behaviour today, not a stub masking a bug - there is
//! nothing to render.

use std::collections::HashMap;
use std::sync::Arc;

use payserver_plugin_api::PluginId;

use super::page::{PageElement, Viewer};

/// What a plugin implements to answer a page request.
///
/// Returns `None` when `path` is not one of the plugin's pages - distinct
/// from the plugin id itself being unregistered, which [`PageHost`] handles
/// before ever calling this.
pub trait PageRenderer: Send + Sync {
    fn render_page(&self, path: &str, viewer: Viewer) -> Option<PageElement>;
}

/// Why a page request was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PageError {
    #[error("plugin not found")]
    PluginNotFound,
    #[error("page not found")]
    PageNotFound,
}

/// The host's page-rendering registry, keyed by plugin id.
///
/// Deliberately separate from [`super::PluginRegistry`]: that registry is
/// the load-time manifest gate, decided once at startup before any plugin
/// code runs. This one is consulted per request and holds the thing that
/// actually produces a page - a different lifecycle, a different concern.
#[derive(Clone, Default)]
pub struct PageHost {
    renderers: HashMap<PluginId, Arc<dyn PageRenderer>>,
}

impl PageHost {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, id: PluginId, renderer: Arc<dyn PageRenderer>) {
        self.renderers.insert(id, renderer);
    }

    /// Resolves a page request. Same method for merchant and admin: the
    /// plugin decides what to return for the [`Viewer`] it is handed, and
    /// this host never substitutes its own judgment for the plugin's - it
    /// only decides who is asking, not what they should see.
    pub fn render(
        &self,
        id: &PluginId,
        path: &str,
        viewer: Viewer,
    ) -> Result<PageElement, PageError> {
        let renderer = self.renderers.get(id).ok_or(PageError::PluginNotFound)?;
        renderer
            .render_page(path, viewer)
            .ok_or(PageError::PageNotFound)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::services::plugins::page::{Badge, Card, Tone};

    fn plugin_id(s: &str) -> PluginId {
        PluginId::new(s).unwrap()
    }

    /// A plugin that answers differently for a merchant and an admin, and
    /// asserts it is never asked to choose - it just returns what it is told
    /// to return for the `Viewer` it is handed.
    struct RoleAwareRenderer;

    impl PageRenderer for RoleAwareRenderer {
        fn render_page(&self, path: &str, viewer: Viewer) -> Option<PageElement> {
            if path != "dashboard" {
                return None;
            }
            let text = match viewer {
                Viewer::Merchant => "Your balance",
                Viewer::Admin => "All merchant balances",
            };
            Some(PageElement::Card(Card {
                title: None,
                children: vec![PageElement::Badge(Badge {
                    text: text.to_string(),
                    tone: Tone::Info,
                })],
            }))
        }
    }

    fn badge_text(element: &PageElement) -> &str {
        let PageElement::Card(card) = element else {
            panic!("expected a card");
        };
        let [PageElement::Badge(badge)] = card.children.as_slice() else {
            panic!("expected exactly one badge child");
        };
        &badge.text
    }

    #[test]
    fn refuses_an_unregistered_plugin() {
        let host = PageHost::new();
        let err = host
            .render(
                &plugin_id("cash.random.billing"),
                "dashboard",
                Viewer::Merchant,
            )
            .unwrap_err();
        assert_eq!(err, PageError::PluginNotFound);
    }

    #[test]
    fn refuses_a_path_the_plugin_does_not_serve() {
        let mut host = PageHost::new();
        host.register(
            plugin_id("cash.random.billing"),
            Arc::new(RoleAwareRenderer),
        );

        let err = host
            .render(
                &plugin_id("cash.random.billing"),
                "not-a-real-page",
                Viewer::Merchant,
            )
            .unwrap_err();
        assert_eq!(err, PageError::PageNotFound);
    }

    /// Ticket requirement: a page requested by a merchant and by an admin
    /// can differ, and the plugin sees the role it was given rather than one
    /// it chose - `PageHost` never inspects or rewrites what comes back.
    #[test]
    fn the_same_plugin_and_path_can_differ_by_viewer() {
        let mut host = PageHost::new();
        host.register(
            plugin_id("cash.random.billing"),
            Arc::new(RoleAwareRenderer),
        );

        let merchant_page = host
            .render(
                &plugin_id("cash.random.billing"),
                "dashboard",
                Viewer::Merchant,
            )
            .unwrap();
        let admin_page = host
            .render(
                &plugin_id("cash.random.billing"),
                "dashboard",
                Viewer::Admin,
            )
            .unwrap();

        assert_eq!(badge_text(&merchant_page), "Your balance");
        assert_eq!(badge_text(&admin_page), "All merchant balances");
        assert_ne!(merchant_page, admin_page);
    }
}
