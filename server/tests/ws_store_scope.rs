#![allow(clippy::unwrap_used, clippy::expect_used)]

//! `/ws` delivers an invoice or payment update only to a socket whose user
//! may see the store it happened in: a server admin, or a member of that
//! store.
//!
//! These tests go through the real `/ws` handler - upgrade, auth frame,
//! session validation, forwarding loop - over a TCP socket, with updates
//! published through the same `WsBroadcast` the event consumers use. Absence
//! is only ever asserted after a positive control: a channel that is simply
//! not delivering would otherwise pass every "does not receive" check.
//!
//! The channel preserves order, so "the first frame is the tenant's own
//! update" also proves that the other tenant's update, published before it,
//! was withheld rather than merely late.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use sqlx::PgPool;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

use auth::{
    Result as AuthResult, Session, SessionId, SessionService, Store, UserId, UserInfo, UserStore,
    UserStoreRepository,
};
use data_service::PgDataService;
use data_service::store_creation::StoreCreationWriter;
use rates::NoOpRateProvider;
use server::api::ws::{StatusUpdate, WsBroadcast, ws_handler};
use server::services::RedisEVMMonitor;
use server::state::PgAppState;

#[allow(dead_code)]
#[path = "cross_tenant_isolation/support.rs"]
mod support;

const WAIT: Duration = Duration::from_secs(5);
const QUIET: Duration = Duration::from_millis(600);

/// Maps a session token to the user it was issued to. The `/ws` handler only
/// asks it who a token belongs to; everything else it decides from the
/// database, which is what these tests are about.
struct Sessions(HashMap<SessionId, UserInfo>);

#[async_trait]
impl SessionService for Sessions {
    async fn validate_session(&self, session_id: SessionId) -> AuthResult<(UserInfo, Session)> {
        match self.0.get(&session_id) {
            Some(user) => Ok((user.clone(), Session::new(user.id, auth::DeviceId::new()))),
            None => Err(auth::AuthError::SessionInvalid),
        }
    }
    async fn logout(&self, _: SessionId) -> AuthResult<()> {
        unimplemented!("not exercised")
    }
    async fn logout_all(&self, _: SessionId) -> AuthResult<()> {
        unimplemented!("not exercised")
    }
    async fn cleanup_stale_sessions(&self) -> AuthResult<u64> {
        unimplemented!("not exercised")
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Person {
    user_id: Uuid,
    session: SessionId,
}

impl Person {
    async fn new(pool: &PgPool) -> Self {
        Self {
            user_id: support::seed_user(pool).await,
            session: SessionId(Uuid::new_v4()),
        }
    }
}

struct Harness {
    addr: SocketAddr,
    broadcast: Arc<WsBroadcast>,
}

async fn serve(data_service: Arc<PgDataService>, people: &[&Person]) -> Harness {
    let sessions = people
        .iter()
        .map(|p| (p.session, support::user_info(p.user_id)))
        .collect();
    let broadcast = Arc::new(WsBroadcast::new(64));
    let mut state = PgAppState::new(
        data_service,
        Arc::new(Sessions(sessions)),
        None::<Arc<RedisEVMMonitor>>,
        Arc::new(NoOpRateProvider),
        Arc::new(server::services::email::NoopEmailSender),
    );
    state.ws_broadcast = Some(broadcast.clone());
    let app = axum::Router::new()
        .route("/ws", axum::routing::get(ws_handler::<Sessions>))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Harness { addr, broadcast }
}

async fn connect(h: &Harness, person: &Person) -> Socket {
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{}/ws", h.addr))
        .await
        .expect("connect");
    let auth = serde_json::json!({"type": "auth", "token": person.session.0.to_string()});
    ws.send(Message::Text(auth.to_string().into()))
        .await
        .unwrap();
    let first = next_update(&mut ws).await.expect("connected frame");
    assert_eq!(first["type"], "connected");
    ws
}

async fn next_update(ws: &mut Socket) -> Option<serde_json::Value> {
    let msg = tokio::time::timeout(WAIT, ws.next()).await.ok()??.ok()?;
    serde_json::from_str(msg.to_text().ok()?).ok()
}

/// Asserts that nothing is delivered within a quiet period.
async fn assert_silent(ws: &mut Socket) {
    if let Ok(Some(Ok(msg))) = tokio::time::timeout(QUIET, ws.next()).await {
        panic!("a frame was delivered that should have been withheld: {msg:?}");
    }
}

fn paid(invoice: &str) -> StatusUpdate {
    StatusUpdate::InvoiceStatus {
        invoice_id: invoice.to_string(),
        status: "paid".to_string(),
    }
}

async fn tenant(pg: &PgDataService, person: &Person, label: &str) -> (Store, UserStore) {
    let store = Store::new(
        format!("ws-{label}-{}", Uuid::new_v4()),
        UserId(person.user_id),
    );
    let membership = pg
        .create_store_owned_by(&store, UserId(person.user_id))
        .await
        .expect("seed store");
    (store, membership)
}

fn sid(store: &Store) -> types::StoreId {
    types::StoreId(store.id.0)
}

async fn make_admin(pool: &PgPool, person: &Person) {
    sqlx::query("UPDATE users SET role = 'server_admin' WHERE id = $1")
        .bind(person.user_id)
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs DATABASE_URL"]
async fn a_tenant_receives_its_own_updates_and_not_another_tenants() {
    let Some(pg) = support::service().await else {
        return;
    };
    let pg = Arc::new(pg);
    let (a, b) = (Person::new(pg.pool()).await, Person::new(pg.pool()).await);
    let (store_a, _) = tenant(&pg, &a, "a").await;
    let (store_b, _) = tenant(&pg, &b, "b").await;
    let h = serve(pg.clone(), &[&a, &b]).await;
    let mut ws_a = connect(&h, &a).await;

    // B's update is published first, then A's.
    h.broadcast.send(sid(&store_b), paid("inv-of-b"));
    h.broadcast.send(sid(&store_a), paid("inv-of-a"));

    let got = next_update(&mut ws_a).await.expect("A's own update");
    assert_eq!(got["invoice_id"], "inv-of-a", "first frame was {got}");
    assert_silent(&mut ws_a).await;
}

#[tokio::test]
#[ignore = "needs DATABASE_URL"]
async fn payment_updates_are_scoped_the_same_way() {
    let Some(pg) = support::service().await else {
        return;
    };
    let pg = Arc::new(pg);
    let (a, b) = (Person::new(pg.pool()).await, Person::new(pg.pool()).await);
    let (store_a, _) = tenant(&pg, &a, "a").await;
    let (store_b, _) = tenant(&pg, &b, "b").await;
    let h = serve(pg.clone(), &[&a, &b]).await;
    let mut ws_a = connect(&h, &a).await;

    let payment = |inv: &str| StatusUpdate::PaymentUpdate {
        payment_id: Uuid::new_v4().to_string(),
        invoice_id: inv.to_string(),
        status: "detected".to_string(),
        amount: Some("1".to_string()),
    };
    h.broadcast.send(sid(&store_b), payment("inv-of-b"));
    h.broadcast.send(sid(&store_a), payment("inv-of-a"));

    let got = next_update(&mut ws_a).await.expect("A's own payment");
    assert_eq!(got["type"], "payment_update");
    assert_eq!(got["invoice_id"], "inv-of-a", "first frame was {got}");
    assert_silent(&mut ws_a).await;
}

#[tokio::test]
#[ignore = "needs DATABASE_URL"]
async fn a_server_admin_receives_every_stores_updates() {
    let Some(pg) = support::service().await else {
        return;
    };
    let pg = Arc::new(pg);
    let (a, b) = (Person::new(pg.pool()).await, Person::new(pg.pool()).await);
    let admin = Person::new(pg.pool()).await;
    make_admin(pg.pool(), &admin).await;
    let (store_a, _) = tenant(&pg, &a, "a").await;
    let (store_b, _) = tenant(&pg, &b, "b").await;
    let h = serve(pg.clone(), &[&admin]).await;
    let mut ws = connect(&h, &admin).await;

    h.broadcast.send(sid(&store_b), paid("inv-of-b"));
    h.broadcast.send(sid(&store_a), paid("inv-of-a"));

    assert_eq!(
        next_update(&mut ws).await.unwrap()["invoice_id"],
        "inv-of-b"
    );
    assert_eq!(
        next_update(&mut ws).await.unwrap()["invoice_id"],
        "inv-of-a"
    );
}

#[tokio::test]
#[ignore = "needs DATABASE_URL"]
async fn a_member_removed_mid_connection_stops_receiving() {
    let Some(pg) = support::service().await else {
        return;
    };
    let pg = Arc::new(pg);
    let owner = Person::new(pg.pool()).await;
    let member = Person::new(pg.pool()).await;
    let admin = Person::new(pg.pool()).await;
    make_admin(pg.pool(), &admin).await;
    let (store, owner_membership) = tenant(&pg, &owner, "shared").await;
    pg.add_user_to_store(&UserStore::new(
        UserId(member.user_id),
        store.id,
        owner_membership.store_role_id,
    ))
    .await
    .unwrap();
    let h = serve(pg.clone(), &[&member, &admin]).await;
    let mut ws_member = connect(&h, &member).await;
    let mut ws_admin = connect(&h, &admin).await;

    // While still a member: delivered. This is the positive control.
    h.broadcast.send(sid(&store), paid("before-removal"));
    assert_eq!(
        next_update(&mut ws_member).await.expect("member sees it")["invoice_id"],
        "before-removal"
    );
    assert_eq!(
        next_update(&mut ws_admin).await.unwrap()["invoice_id"],
        "before-removal"
    );

    pg.remove_user_from_store(UserId(member.user_id), store.id)
        .await
        .unwrap();

    // The socket is still open; the very next event must not reach it.
    h.broadcast.send(sid(&store), paid("after-removal"));
    assert_eq!(
        next_update(&mut ws_admin).await.unwrap()["invoice_id"],
        "after-removal",
        "the event was published and reached an entitled socket"
    );
    assert_silent(&mut ws_member).await;
}

#[tokio::test]
#[ignore = "needs DATABASE_URL"]
async fn when_the_decision_cannot_be_made_nothing_is_delivered() {
    let Some(pg) = support::service().await else {
        return;
    };
    let url = std::env::var("DATABASE_URL").unwrap();
    // The handler gets its own pool so that closing it breaks only the
    // handler's reads, not the seeding done through the shared one.
    let handler_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    let a = Person::new(pg.pool()).await;
    let (store_a, _) = tenant(&pg, &a, "a").await;
    let h = serve(Arc::new(PgDataService::new(handler_pool.clone())), &[&a]).await;
    let mut ws_a = connect(&h, &a).await;
    let mut observer = h.broadcast.subscribe();

    h.broadcast.send(sid(&store_a), paid("healthy"));
    assert_eq!(
        next_update(&mut ws_a)
            .await
            .expect("delivered while healthy")["invoice_id"],
        "healthy"
    );

    handler_pool.close().await;

    h.broadcast.send(sid(&store_a), paid("db-down"));
    // The update was published, and is the owner's own: only the failed
    // check stands between it and the socket.
    let seen_by_channel = loop {
        let event = tokio::time::timeout(WAIT, observer.recv())
            .await
            .expect("channel delivers")
            .unwrap();
        if matches!(&event.update, StatusUpdate::InvoiceStatus { invoice_id, .. } if invoice_id == "db-down")
        {
            break event;
        }
    };
    assert_eq!(seen_by_channel.store_id, sid(&store_a));
    assert_silent(&mut ws_a).await;
}

#[tokio::test]
#[ignore = "needs DATABASE_URL"]
async fn an_update_for_an_invoice_is_attributed_to_the_invoices_store() {
    let Some(pg) = support::service().await else {
        return;
    };
    let (a, b) = (
        support::seed_tenant(&pg, "a").await,
        support::seed_tenant(&pg, "b").await,
    );
    let broadcast = WsBroadcast::new(16);
    let mut observer = broadcast.subscribe();

    // An invoice that does not exist has no store, so nothing is published.
    broadcast
        .send_for_invoice(&pg, &types::InvoiceId::new(), paid("ghost"))
        .await;
    broadcast
        .send_for_invoice(&pg, &b.invoice.id, paid("b"))
        .await;
    broadcast
        .send_for_invoice(&pg, &a.invoice.id, paid("a"))
        .await;

    let first = observer.recv().await.unwrap();
    assert_eq!(first.store_id, types::StoreId(b.store.id.0));
    let second = observer.recv().await.unwrap();
    assert_eq!(second.store_id, types::StoreId(a.store.id.0));
    assert!(observer.try_recv().is_err(), "nothing else was published");
}
