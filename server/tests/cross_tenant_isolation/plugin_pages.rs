//! Plugin pages: viewer and account come from the session, never the request.
//!
//! Review finding, checked: `get_page` is mounted at
//! `GET /plugins/{id}/pages/{*path}` (`server/src/api/mod.rs`) - not an
//! orphaned handler.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::extract::{Path, State};
use uuid::Uuid;

use data_service::PgDataService;
use payserver_plugin_api::PluginId;
use payserver_plugin_host::{PageHost, PageRenderError, PageRenderer};
use server::api::AuthenticatedUser;
use server::services::plugins::{PageElement, PageRequest, Viewer};

use crate::support::{app_state, user_info, user_info_with_role};

/// A `render_page` implementation that records every request it is handed,
/// so the test can check what the host told the plugin rather than trusting
/// a doc comment.
struct RecordingRenderer(Arc<Mutex<Vec<PageRequest>>>);

#[async_trait]
impl PageRenderer for RecordingRenderer {
    async fn render_page(
        &self,
        request: &PageRequest,
    ) -> Result<Option<PageElement>, PageRenderError> {
        self.0.lock().unwrap().push(request.clone());
        Ok(None)
    }
}

/// The billing plugin's merchant/admin split depends on `render_page` being
/// told the truth about who is asking. `get_page` resolves `viewer` and
/// `account_id` only from the authenticated `UserInfo` it is handed - never
/// from the path or query - so two different callers must never be recorded
/// as the same account, each must see their own identity, not the other
/// one's, and a `ServerAdmin` caller must be recorded as `Viewer::Admin`,
/// not `Viewer::Merchant`.
///
/// No real Postgres needed: `resolve_plugin`'s lookup fails against the
/// lazily-connecting pool and falls back to treating the path segment as a
/// literal plugin id, exactly as `server/src/api/plugins.rs`'s own
/// `a_registered_renderer_is_reachable_over_http` test relies on. The pool
/// points at the reserved `.invalid` TLD (RFC 2606) rather than `localhost`,
/// so the lookup fails on DNS resolution alone - no environment can make it
/// succeed by happening to have a database of that name reachable locally,
/// which would silently swap this test onto a different code path than the
/// one it means to cover.
#[tokio::test]
async fn plugin_page_viewer_and_account_are_always_the_callers_own() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let plugin_id = PluginId::new("cash.random.billing").unwrap();

    let mut pages = PageHost::new();
    pages.register(plugin_id, Arc::new(RecordingRenderer(seen.clone())));

    let pool = sqlx::PgPool::connect_lazy("postgres://nonexistent.invalid/nonexistent").unwrap();
    let mut state = app_state(Arc::new(PgDataService::new(pool)));
    state.plugin_pages = Arc::new(pages);

    let account_a = Uuid::new_v4();
    let account_b = Uuid::new_v4();
    let account_admin = Uuid::new_v4();

    let _ = server::api::plugins::get_page(
        State(state.clone()),
        AuthenticatedUser(user_info(account_a)),
        Path((
            "cash.random.billing".to_string(),
            "subscriptions".to_string(),
        )),
    )
    .await;
    let _ = server::api::plugins::get_page(
        State(state.clone()),
        AuthenticatedUser(user_info(account_b)),
        Path((
            "cash.random.billing".to_string(),
            "subscriptions".to_string(),
        )),
    )
    .await;
    let _ = server::api::plugins::get_page(
        State(state),
        AuthenticatedUser(user_info_with_role(account_admin, auth::Role::ServerAdmin)),
        Path((
            "cash.random.billing".to_string(),
            "subscriptions".to_string(),
        )),
    )
    .await;

    let seen = seen.lock().unwrap();
    assert_eq!(
        seen.len(),
        3,
        "all three requests must have reached the renderer"
    );
    assert_eq!(seen[0].viewer, Viewer::Merchant);
    assert_eq!(
        seen[0].account_id.as_deref(),
        Some(account_a.to_string()).as_deref()
    );
    assert_eq!(seen[1].viewer, Viewer::Merchant);
    assert_eq!(
        seen[1].account_id.as_deref(),
        Some(account_b.to_string()).as_deref()
    );
    assert_ne!(
        seen[0].account_id, seen[1].account_id,
        "two different callers must never be recorded under the same account"
    );
    assert_eq!(
        seen[2].viewer,
        Viewer::Admin,
        "a ServerAdmin caller must be recorded as the admin viewer, not the merchant one"
    );
}
