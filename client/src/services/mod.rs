//! Client-side services.

pub mod websocket;

pub use websocket::{ConnectionState, StatusUpdate, WebSocketService};
