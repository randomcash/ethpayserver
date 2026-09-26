use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::PluginCalls;

/// The same late-binding cell for capability 6.
///
/// Separate from [`DeferredIssuer`](super::DeferredIssuer) rather than one cell holding both,
/// because the two are available under different conditions: issuing needs
/// the instance's own store and reading a merchant's volume does not. Sharing
/// a cell would make an instance with no billing store silently unable to
/// answer a question it can answer perfectly well.
#[derive(Clone, Default)]
pub struct DeferredVolume(
    Arc<std::sync::OnceLock<Arc<dyn crate::services::plugins::MerchantVolumeReader>>>,
);

impl DeferredVolume {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Publish the reader. Returns whether this call is the one that set it.
    pub fn publish(&self, reader: Arc<dyn crate::services::plugins::MerchantVolumeReader>) -> bool {
        self.0.set(reader).is_ok()
    }

    fn get(&self) -> Option<&Arc<dyn crate::services::plugins::MerchantVolumeReader>> {
        self.0.get()
    }
}

impl std::fmt::Debug for DeferredVolume {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("DeferredVolume")
            .field(&self.get().is_some())
            .finish()
    }
}

/// What a plugin quotes volume in when it does not say.
///
/// Named rather than defaulted silently: a plugin that omits the currency is
/// asking for "the usual", and the usual on this instance is the unit its
/// brackets are written in.
const DEFAULT_VOLUME_CURRENCY: &str = "USD";

/// A plugin asking what one account settled.
#[derive(Debug, Deserialize)]
struct VolumeRequest {
    account_id: String,
    /// How far back to sum. Clamped host-side - see
    /// [`MAX_WINDOW_DAYS`](crate::services::plugins::MAX_WINDOW_DAYS) - because the plugin names
    /// it.
    window_days: u32,
    #[serde(default)]
    currency: String,
}

/// The answer: one number, and what could not be counted towards it.
#[derive(Debug, Serialize)]
struct VolumeAnswer {
    /// A decimal string. Every value crossing this boundary is text - a JSON
    /// number cannot carry what these columns hold.
    volume: String,
    currency: String,
    /// Assets present in the window that could not be priced, and are
    /// therefore missing from `volume`. Their absence makes the answer an
    /// undercount, which can only under-bill.
    unpriced_assets: Vec<String>,
}

impl PluginCalls {
    pub(super) fn merchant_volume_impl(&self, request: &[u8]) -> Result<Vec<u8>, String> {
        let parsed: VolumeRequest = serde_json::from_slice(request)
            .map_err(|e| format!("could not read the volume request: {e}"))?;

        let answer = self.read_volume(&parsed)?;
        serde_json::to_vec(&answer)
            .map_err(|e| format!("could not serialise the volume answer: {e}"))
    }

    fn read_volume(&self, request: &VolumeRequest) -> Result<VolumeAnswer, String> {
        let Some(reader) = self.volume.get().cloned() else {
            return Err("this host does not report merchant volume".to_string());
        };

        // The account is parsed here rather than passed through as text, so
        // an id this instance could never have issued is refused before it
        // reaches a query. The plugin holds the same string the page request
        // handed it, which is a `UserId` rendered - anything else is either a
        // bug in the plugin or a plugin asking about something it made up.
        let account_id = uuid::Uuid::parse_str(&request.account_id)
            .map(types::UserId)
            .map_err(|_| format!("{} is not an account id", request.account_id))?;

        let currency = if request.currency.trim().is_empty() {
            DEFAULT_VOLUME_CURRENCY
        } else {
            request.currency.trim()
        };

        let volume = self.handle.block_on(reader.merchant_volume(
            account_id,
            request.window_days,
            currency,
        ))?;

        Ok(VolumeAnswer {
            volume: volume.volume,
            currency: volume.currency,
            unpriced_assets: volume.unpriced_assets,
        })
    }
}

#[cfg(test)]
mod tests;
