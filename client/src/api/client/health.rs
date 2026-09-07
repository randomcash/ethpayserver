//! Health API methods.

use super::{ApiError, EvmApiClient};
use crate::api::ChainsHealthResponse;

impl EvmApiClient {
    /// Fetch per-chain monitor health.
    ///
    /// Server admins only: any other caller gets `403`. The server answers
    /// `503` when the monitor has published nothing to Redis, so an error here
    /// is a real "we do not know" and must not be rendered as healthy — a
    /// green panel over dead monitors was RCS-196.
    ///
    /// `/health` is exempt from the IP rate limit tiers
    /// (`server/src/api/rate_limit.rs`), so polling this is safe.
    pub async fn get_chains_health(&self) -> Result<ChainsHealthResponse, ApiError> {
        self.get("/health/chains").await
    }
}
