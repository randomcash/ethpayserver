//! Test utilities for data service.

use std::collections::HashMap;
use std::sync::RwLock;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::stream::{self, BoxStream, StreamExt};
use types::{
    CleanupAddressInfo, InvoiceData, InvoiceId, InvoiceQueryParams, InvoiceReader, InvoiceStatus,
    InvoiceWriter, Network, PaymentData, PaymentEventWriter, PaymentReader, PaymentWriter,
    PendingWatchInfo, RepositoryResult, StoreId, StoreWebhook, StoreWebhookReader, TokenData,
    TokenQueryParams, TokenReader, TokenWriter, WatchedAddressReader, WatchedAddressWriter,
};
use uuid::Uuid;

/// In-memory implementation of all repository traits for testing.
#[derive(Default)]
pub struct InMemoryDataService {
    invoices: RwLock<HashMap<String, InvoiceData>>,
    payments: RwLock<HashMap<Uuid, PaymentData>>,
    addresses: RwLock<HashMap<(String, Network), InvoiceId>>,
    tokens: RwLock<HashMap<i64, TokenData>>,
    token_id_counter: RwLock<i64>,
    webhooks: RwLock<HashMap<Uuid, StoreWebhook>>,
}

impl InMemoryDataService {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set up a webhook for a store (for testing).
    pub fn set_webhook(&self, store_id: Uuid, url: &str, secret: &str) {
        let mut webhooks = self.webhooks.write().unwrap();
        webhooks.insert(
            store_id,
            StoreWebhook {
                id: Uuid::new_v4(),
                store_id,
                webhook_url: url.to_string(),
                webhook_secret: secret.to_string(),
                enabled: true,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
        );
    }
}

#[async_trait]
impl InvoiceReader for InMemoryDataService {
    async fn get(&self, id: &InvoiceId) -> RepositoryResult<Option<InvoiceData>> {
        let invoices = self.invoices.read().unwrap();
        Ok(invoices.get(&id.0).cloned())
    }

    async fn query(
        &self,
        params: &InvoiceQueryParams,
    ) -> RepositoryResult<(i64, Vec<InvoiceData>)> {
        let invoices = self.invoices.read().unwrap();
        let mut results: Vec<InvoiceData> = invoices
            .values()
            .filter(|inv| {
                if let Some(store_id) = params.store_id {
                    if inv.store_id != store_id {
                        return false;
                    }
                }
                if let Some(status) = params.status {
                    if inv.status != status {
                        return false;
                    }
                }
                if let Some(network) = params.network {
                    if inv.network != network {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();

        let total = results.len() as i64;
        results.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        let offset = params.offset as usize;
        let limit = params.limit as usize;
        let results = results.into_iter().skip(offset).take(limit).collect();

        Ok((total, results))
    }

    async fn get_expired(&self) -> RepositoryResult<Vec<InvoiceData>> {
        let invoices = self.invoices.read().unwrap();
        let now = Utc::now();
        Ok(invoices
            .values()
            .filter(|inv| inv.status == InvoiceStatus::Pending && inv.expires_at < now)
            .cloned()
            .collect())
    }

    fn stream_expired_pending_for_network(
        &self,
        network: Network,
    ) -> BoxStream<'_, RepositoryResult<InvoiceId>> {
        let invoices = self.invoices.read().unwrap();
        let now = Utc::now();
        let ids: Vec<RepositoryResult<InvoiceId>> = invoices
            .values()
            .filter(|inv| {
                inv.status == InvoiceStatus::Pending
                    && inv.network == network
                    && inv.expires_at < now
            })
            .map(|inv| Ok(inv.id.clone()))
            .collect();
        stream::iter(ids).boxed()
    }

    fn stream_all_expired_pending(&self) -> BoxStream<'_, RepositoryResult<(Network, InvoiceId)>> {
        let invoices = self.invoices.read().unwrap();
        let now = Utc::now();
        let ids: Vec<RepositoryResult<(Network, InvoiceId)>> = invoices
            .values()
            .filter(|inv| inv.status == InvoiceStatus::Pending && inv.expires_at < now)
            .map(|inv| Ok((inv.network, inv.id.clone())))
            .collect();
        stream::iter(ids).boxed()
    }
}

#[async_trait]
impl InvoiceWriter for InMemoryDataService {
    async fn upsert(&self, invoice: &InvoiceData) -> RepositoryResult<()> {
        let mut invoices = self.invoices.write().unwrap();
        invoices.insert(invoice.id.0.clone(), invoice.clone());
        Ok(())
    }

    async fn update_status(&self, id: &InvoiceId, status: InvoiceStatus) -> RepositoryResult<()> {
        let mut invoices = self.invoices.write().unwrap();
        if let Some(invoice) = invoices.get_mut(&id.0) {
            invoice.status = status;
        }
        Ok(())
    }

    async fn update_amount_received(&self, id: &InvoiceId, amount: &str) -> RepositoryResult<()> {
        let mut invoices = self.invoices.write().unwrap();
        if let Some(invoice) = invoices.get_mut(&id.0) {
            invoice.amount_received = amount.to_string();
        }
        Ok(())
    }

    async fn expire(&self, id: &InvoiceId) -> RepositoryResult<bool> {
        let mut invoices = self.invoices.write().unwrap();
        if let Some(invoice) = invoices.get_mut(&id.0) {
            if invoice.status == InvoiceStatus::Pending {
                invoice.status = InvoiceStatus::Expired;
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[async_trait]
impl PaymentReader for InMemoryDataService {
    async fn get(&self, id: Uuid) -> RepositoryResult<Option<PaymentData>> {
        let payments = self.payments.read().unwrap();
        Ok(payments.get(&id).cloned())
    }

    async fn get_for_invoice(&self, invoice_id: &InvoiceId) -> RepositoryResult<Vec<PaymentData>> {
        let payments = self.payments.read().unwrap();
        Ok(payments
            .values()
            .filter(|p| p.invoice_id == *invoice_id)
            .cloned()
            .collect())
    }

    async fn get_awaiting_confirmation(&self) -> RepositoryResult<Vec<PaymentData>> {
        let payments = self.payments.read().unwrap();
        Ok(payments
            .values()
            .filter(|p| p.confirmed_at.is_none() && !p.reorged)
            .cloned()
            .collect())
    }

    async fn get_valid_for_invoice(&self, invoice_id: &InvoiceId) -> RepositoryResult<Vec<PaymentData>> {
        let payments = self.payments.read().unwrap();
        Ok(payments
            .values()
            .filter(|p| p.invoice_id == *invoice_id && !p.reorged)
            .cloned()
            .collect())
    }

    async fn has_valid_payments(&self, invoice_id: &InvoiceId) -> RepositoryResult<bool> {
        let payments = self.payments.read().unwrap();
        Ok(payments
            .values()
            .any(|p| p.invoice_id == *invoice_id && !p.reorged))
    }
}

#[async_trait]
impl PaymentWriter for InMemoryDataService {
    async fn upsert(&self, payment: &PaymentData) -> RepositoryResult<()> {
        let mut payments = self.payments.write().unwrap();
        payments.insert(payment.id, payment.clone());
        Ok(())
    }

    async fn mark_confirmed(&self, id: Uuid, confirmed_at: DateTime<Utc>) -> RepositoryResult<()> {
        let mut payments = self.payments.write().unwrap();
        if let Some(payment) = payments.get_mut(&id) {
            if payment.confirmed_at.is_none() {
                payment.confirmed_at = Some(confirmed_at);
            }
        }
        Ok(())
    }

    async fn mark_reorged(
        &self,
        invoice_id: &InvoiceId,
        network: Network,
        fork_block: u64,
    ) -> RepositoryResult<u64> {
        let mut payments = self.payments.write().unwrap();
        let mut count = 0u64;
        for payment in payments.values_mut() {
            if payment.invoice_id == *invoice_id
                && payment.network == network
                && payment.block_number.is_some_and(|b| b >= fork_block)
                && !payment.reorged
            {
                payment.reorged = true;
                payment.confirmed_at = None;
                count += 1;
            }
        }
        Ok(count)
    }
}

#[async_trait]
impl WatchedAddressReader for InMemoryDataService {
    async fn get_invoice_id(
        &self,
        address: &str,
        network: Network,
    ) -> RepositoryResult<Option<InvoiceId>> {
        let addresses = self.addresses.read().unwrap();
        Ok(addresses.get(&(address.to_string(), network)).cloned())
    }

    async fn get_active(&self) -> RepositoryResult<Vec<(String, InvoiceId, Network)>> {
        let addresses = self.addresses.read().unwrap();
        Ok(addresses
            .iter()
            .map(|((addr, network), id)| (addr.clone(), id.clone(), *network))
            .collect())
    }

    async fn get_pending(&self) -> RepositoryResult<Vec<PendingWatchInfo>> {
        let addresses = self.addresses.read().unwrap();
        Ok(addresses
            .iter()
            .map(|((addr, network), id)| PendingWatchInfo {
                address: addr.clone(),
                invoice_id: id.as_str().to_string(),
                network: *network,
                expected_amount: None,
                asset_id: None,
            })
            .collect())
    }

    async fn get_expired_for_cleanup(
        &self,
        _grace_period_secs: i64,
    ) -> RepositoryResult<Vec<CleanupAddressInfo>> {
        // For in-memory testing, we would need to join with invoices.
        // Return empty for simplicity - real DB implementation handles this.
        let addresses = self.addresses.read().unwrap();
        let invoices = self.invoices.read().unwrap();
        let now = Utc::now();

        Ok(addresses
            .iter()
            .filter_map(|((addr, network), invoice_id)| {
                invoices.get(&invoice_id.0).and_then(|inv| {
                    if inv.status == InvoiceStatus::Expired && inv.expires_at < now {
                        Some(CleanupAddressInfo {
                            address: addr.clone(),
                            network: *network,
                            invoice_id: invoice_id.as_str().to_string(),
                        })
                    } else {
                        None
                    }
                })
            })
            .collect())
    }

    async fn get_paid_for_cleanup(&self) -> RepositoryResult<Vec<CleanupAddressInfo>> {
        let addresses = self.addresses.read().unwrap();
        let invoices = self.invoices.read().unwrap();

        Ok(addresses
            .iter()
            .filter_map(|((addr, network), invoice_id)| {
                invoices.get(&invoice_id.0).and_then(|inv| {
                    if inv.status == InvoiceStatus::Paid {
                        Some(CleanupAddressInfo {
                            address: addr.clone(),
                            network: *network,
                            invoice_id: invoice_id.as_str().to_string(),
                        })
                    } else {
                        None
                    }
                })
            })
            .collect())
    }

    async fn get_cancelled_for_cleanup(&self) -> RepositoryResult<Vec<CleanupAddressInfo>> {
        let addresses = self.addresses.read().unwrap();
        let invoices = self.invoices.read().unwrap();

        Ok(addresses
            .iter()
            .filter_map(|((addr, network), invoice_id)| {
                invoices.get(&invoice_id.0).and_then(|inv| {
                    if inv.status == InvoiceStatus::Cancelled {
                        Some(CleanupAddressInfo {
                            address: addr.clone(),
                            network: *network,
                            invoice_id: invoice_id.as_str().to_string(),
                        })
                    } else {
                        None
                    }
                })
            })
            .collect())
    }
}

#[async_trait]
impl WatchedAddressWriter for InMemoryDataService {
    async fn upsert(
        &self,
        address: &str,
        invoice_id: &InvoiceId,
        network: Network,
    ) -> RepositoryResult<()> {
        let mut addresses = self.addresses.write().unwrap();
        addresses.insert((address.to_string(), network), invoice_id.clone());
        Ok(())
    }

    async fn remove(&self, address: &str, network: Network) -> RepositoryResult<()> {
        let mut addresses = self.addresses.write().unwrap();
        addresses.remove(&(address.to_string(), network));
        Ok(())
    }

    async fn upsert_with_asset(
        &self,
        address: &str,
        invoice_id: &InvoiceId,
        network: Network,
        _asset_id: Option<&str>,
    ) -> RepositoryResult<()> {
        // Simplified: ignore asset_id for in-memory testing
        let mut addresses = self.addresses.write().unwrap();
        addresses.insert((address.to_string(), network), invoice_id.clone());
        Ok(())
    }

    async fn mark_notified(&self, _address: &str, _network: Network) -> RepositoryResult<()> {
        // No-op for in-memory testing
        Ok(())
    }

    async fn deactivate(&self, address: &str, network: Network) -> RepositoryResult<bool> {
        let mut addresses = self.addresses.write().unwrap();
        let key = (address.to_string(), network);
        if addresses.remove(&key).is_some() {
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

#[async_trait]
impl TokenReader for InMemoryDataService {
    async fn get(&self, id: i64) -> RepositoryResult<Option<TokenData>> {
        let tokens = self.tokens.read().unwrap();
        Ok(tokens.get(&id).cloned())
    }

    async fn get_by_address(
        &self,
        network: Network,
        address: &str,
    ) -> RepositoryResult<Option<TokenData>> {
        let tokens = self.tokens.read().unwrap();
        Ok(tokens
            .values()
            .find(|t| t.network == network && t.address.eq_ignore_ascii_case(address))
            .cloned())
    }

    async fn find_by_symbol(
        &self,
        network: Network,
        symbol: &str,
    ) -> RepositoryResult<Option<TokenData>> {
        let tokens = self.tokens.read().unwrap();
        Ok(tokens
            .values()
            .find(|t| {
                t.network == network
                    && t.symbol
                        .as_ref()
                        .is_some_and(|s| s.eq_ignore_ascii_case(symbol))
            })
            .cloned())
    }

    async fn query(&self, params: &TokenQueryParams) -> RepositoryResult<(i64, Vec<TokenData>)> {
        let tokens = self.tokens.read().unwrap();
        let mut results: Vec<TokenData> = tokens
            .values()
            .filter(|t| {
                if let Some(ref token_type) = params.token_type {
                    if t.token_type != *token_type {
                        return false;
                    }
                }
                if let Some(network) = params.network {
                    if t.network != network {
                        return false;
                    }
                }
                if let Some(enabled) = params.enabled {
                    if t.enabled != enabled {
                        return false;
                    }
                }
                if let Some(ref symbol) = params.symbol {
                    if !t
                        .symbol
                        .as_ref()
                        .is_some_and(|s| s.eq_ignore_ascii_case(symbol))
                    {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();

        let total = results.len() as i64;
        results.sort_by(|a, b| {
            a.network
                .display_name()
                .cmp(b.network.display_name())
                .then_with(|| a.symbol.cmp(&b.symbol))
        });

        let offset = params.offset as usize;
        let limit = params.limit as usize;
        let results = results.into_iter().skip(offset).take(limit).collect();

        Ok((total, results))
    }

    async fn get_enabled_for_network(&self, network: Network) -> RepositoryResult<Vec<TokenData>> {
        let tokens = self.tokens.read().unwrap();
        Ok(tokens
            .values()
            .filter(|t| t.network == network && t.enabled)
            .cloned()
            .collect())
    }
}

#[async_trait]
impl TokenWriter for InMemoryDataService {
    async fn insert(&self, token: &TokenData) -> RepositoryResult<i64> {
        let mut tokens = self.tokens.write().unwrap();
        let mut counter = self.token_id_counter.write().unwrap();
        *counter += 1;
        let id = *counter;

        let mut token = token.clone();
        token.id = Some(id);
        tokens.insert(id, token);

        Ok(id)
    }

    async fn update(&self, token: &TokenData) -> RepositoryResult<()> {
        let id = token.id.ok_or_else(|| {
            types::RepositoryError::InvalidData("Token must have an ID for update".into())
        })?;

        let mut tokens = self.tokens.write().unwrap();
        if tokens.contains_key(&id) {
            tokens.insert(id, token.clone());
            Ok(())
        } else {
            Err(types::RepositoryError::NotFound(format!(
                "Token {} not found",
                id
            )))
        }
    }

    async fn delete(&self, id: i64) -> RepositoryResult<()> {
        let mut tokens = self.tokens.write().unwrap();
        if tokens.remove(&id).is_some() {
            Ok(())
        } else {
            Err(types::RepositoryError::NotFound(format!(
                "Token {} not found",
                id
            )))
        }
    }

    async fn set_enabled(&self, id: i64, enabled: bool) -> RepositoryResult<()> {
        let mut tokens = self.tokens.write().unwrap();
        if let Some(token) = tokens.get_mut(&id) {
            token.enabled = enabled;
            Ok(())
        } else {
            Err(types::RepositoryError::NotFound(format!(
                "Token {} not found",
                id
            )))
        }
    }
}

#[async_trait]
impl StoreWebhookReader for InMemoryDataService {
    async fn get_webhook(&self, store_id: Uuid) -> RepositoryResult<Option<StoreWebhook>> {
        let webhooks = self.webhooks.read().unwrap();
        Ok(webhooks.get(&store_id).cloned())
    }

    async fn get_enabled_webhook(&self, store_id: Uuid) -> RepositoryResult<Option<StoreWebhook>> {
        let webhooks = self.webhooks.read().unwrap();
        Ok(webhooks
            .get(&store_id)
            .filter(|w| w.enabled)
            .cloned())
    }
}

#[async_trait]
impl PaymentEventWriter for InMemoryDataService {
    async fn create_event(
        &self,
        _invoice_id: &InvoiceId,
        _payment_id: Option<Uuid>,
        _event_type: &str,
        _event_data: Option<serde_json::Value>,
    ) -> RepositoryResult<Uuid> {
        // No-op for tests - just return a new UUID
        Ok(Uuid::new_v4())
    }
}

/// Create a test invoice.
pub fn create_test_invoice() -> InvoiceData {
    InvoiceData {
        id: InvoiceId::new(),
        store_id: StoreId::new(),
        network: Network::Ethereum,
        status: InvoiceStatus::Pending,
        amount: "1000000000000000000".to_string(), // 1 ETH in wei
        amount_received: "0".to_string(),
        asset_symbol: "ETH".to_string(),
        payment_address: Some("0x1234567890abcdef1234567890abcdef12345678".to_string()),
        payment_request: None,
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        metadata: None,
        extra: None,
    }
}

/// Create a test payment.
pub fn create_test_payment(invoice_id: &InvoiceId) -> PaymentData {
    PaymentData {
        id: Uuid::new_v4(),
        invoice_id: invoice_id.clone(),
        network: Network::Ethereum,
        asset_type: types::AssetType::Native,
        amount: "1000000000000000000".to_string(),
        asset_symbol: "ETH".to_string(),
        token_address: None,
        tx_hash: "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890".to_string(),
        block_number: Some(12345678),
        detected_at: Utc::now(),
        confirmed_at: None,
        from_address: Some("0xabcdef1234567890abcdef1234567890abcdef12".to_string()),
        reorged: false,
        extra: None,
    }
}
