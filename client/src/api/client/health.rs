//! Health API methods.

use super::{ApiError, EvmApiClient};
use crate::api::ChainsHealthResponse;

impl EvmApiClient {
    /// Fetch per-chain monitor health.
    ///
    /// Answers every caller. An admin additionally gets block heights, the
    /// watched-address count and the reason behind a failure; everyone else
    /// gets the chain's identity and whether it is up, which is the part a
    /// merchant needs.
    ///
    /// The server answers `503` when the monitor has published nothing to
    /// Redis, so an error here is a real "we do not know" and must not be
    /// rendered as healthy — a green panel over dead monitors was RCS-196.
    ///
    /// `/health` is exempt from the IP rate limit tiers
    /// (`server/src/api/rate_limit.rs`), so polling this is safe.
    pub async fn get_chains_health(&self) -> Result<ChainsHealthResponse, ApiError> {
        self.get("/health/chains").await
    }
}
