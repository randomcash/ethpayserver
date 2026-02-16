//! API client for ethpayserver.

mod client;
mod types;

pub use client::{ApiError, EvmApiClient};
pub use types::*;
