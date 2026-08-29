//! Shared test doubles for the MCP server.
//!
//! Everything here is in-process: an in-memory data service, a stub auth
//! repository, and a stub rate provider. No test touches Postgres, Redis, or a
//! rate API.

mod auth_repo;
mod harness;
mod rates;

pub use auth_repo::{RAW_KEY, StubAuthRepo, test_api_key, test_store};
pub use harness::{CHAIN_ID, TestHarness, parse_ok};
pub use rates::{StubRateProvider, USD_TO_ETH};
