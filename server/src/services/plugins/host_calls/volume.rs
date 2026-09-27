use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::PluginCalls;

/// The same late-binding cell for capability 6.
///
/// Separate from [`DeferredIssuer`](super::DeferredIssuer) rather than one cell holding both,
/// because the two are available under different conditions: issuing needs
/// the instance's own store and reading a merchant's volume does not. Sharing
/// a cell would make an instance with no operator store silently unable to
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

/// The same late-binding cell for the batched form of capability 6.
///
/// Its own cell rather than reusing [`DeferredVolume`]'s, for the same
/// reason that one is not folded into [`DeferredIssuer`](super::DeferredIssuer):
/// nothing requires the two to be published together, even though in practice
/// an instance that can answer for one account can answer for many.
#[derive(Clone, Default)]
pub struct DeferredBulkVolume(
    Arc<std::sync::OnceLock<Arc<dyn crate::services::plugins::BulkMerchantVolumeReader>>>,
);

impl DeferredBulkVolume {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Publish the reader. Returns whether this call is the one that set it.
    pub fn publish(
        &self,
        reader: Arc<dyn crate::services::plugins::BulkMerchantVolumeReader>,
    ) -> bool {
        self.0.set(reader).is_ok()
    }

    fn get(&self) -> Option<&Arc<dyn crate::services::plugins::BulkMerchantVolumeReader>> {
        self.0.get()
    }
}

impl std::fmt::Debug for DeferredBulkVolume {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("DeferredBulkVolume")
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

/// A plugin asking what many accounts settled, in one call. The batched form
/// of [`VolumeRequest`]: a list of accounts in place of one, everything else
/// the same.
#[derive(Debug, Deserialize)]
struct BulkVolumeRequest {
    account_ids: Vec<String>,
    /// See [`VolumeRequest::window_days`].
    window_days: u32,
    #[serde(default)]
    currency: String,
}

/// The batched answer: one entry per requested account.
#[derive(Debug, Serialize)]
struct BulkVolumeAnswer {
    accounts: Vec<AccountVolumeAnswer>,
}

/// [`VolumeAnswer`] with the account it belongs to named alongside it, since
/// the answer is a list rather than one value the caller already knows the
/// subject of.
#[derive(Debug, Serialize)]
struct AccountVolumeAnswer {
    account_id: String,
    volume: String,
    currency: String,
    unpriced_assets: Vec<String>,
}

impl PluginCalls {
    pub(super) fn merchant_volumes_impl(&self, request: &[u8]) -> Result<Vec<u8>, String> {
        let parsed: BulkVolumeRequest = serde_json::from_slice(request)
            .map_err(|e| format!("could not read the volume request: {e}"))?;

        let answer = self.read_volumes(&parsed)?;
        serde_json::to_vec(&answer)
            .map_err(|e| format!("could not serialise the volume answer: {e}"))
    }

    fn read_volumes(&self, request: &BulkVolumeRequest) -> Result<BulkVolumeAnswer, String> {
        let Some(reader) = self.bulk_volume.get().cloned() else {
            return Err("this host does not report merchant volume".to_string());
        };

        // Same rule as `read_volume`: every id is parsed before any of them
        // reaches a query, so a batch with one made-up account refuses the
        // whole call rather than silently answering for the rest.
        let account_ids = request
            .account_ids
            .iter()
            .map(|id| {
                uuid::Uuid::parse_str(id)
                    .map(types::UserId)
                    .map_err(|_| format!("{id} is not an account id"))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let currency = if request.currency.trim().is_empty() {
            DEFAULT_VOLUME_CURRENCY
        } else {
            request.currency.trim()
        };

        let volumes = self.handle.block_on(reader.merchant_volumes(
            &account_ids,
            request.window_days,
            currency,
        ))?;

        Ok(BulkVolumeAnswer {
            accounts: volumes
                .into_iter()
                .map(|account| AccountVolumeAnswer {
                    account_id: account.account_id.0.to_string(),
                    volume: account.volume.volume,
                    currency: account.volume.currency,
                    unpriced_assets: account.volume.unpriced_assets,
                })
                .collect(),
        })
    }

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
