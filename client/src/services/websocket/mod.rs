//! WebSocket service for real-time invoice and payment status updates.
//!
//! [`types`] holds the wire messages and connection state, [`backoff`] the
//! reconnect delay schedule, [`service`] the [`WebSocketService`] handle held
//! in Leptos context, and [`reconnect`] the standalone socket used by the
//! automatic reconnect path.

mod backoff;
mod reconnect;
mod service;
mod types;

pub use service::WebSocketService;
pub use types::{ConnectionState, StatusUpdate};
