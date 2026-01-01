//! Test utilities for data service.

use std::collections::HashMap;
use std::sync::RwLock;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::stream::{self, BoxStream, StreamExt};
use types::{
    CleanupAddressInfo, InvoiceData, InvoiceId, InvoiceQueryParams, InvoiceReader, InvoiceStatus,
    InvoiceWriter, Network, PaymentData, PaymentEventWriter, PaymentMethodId, PaymentOptionData,
    PaymentOptionId, PaymentOptionReader, PaymentOptionWriter, PaymentReader, PaymentWriter,
    PendingWatchInfo, RepositoryResult, StoreId, StoreWebhook, StoreWebhookReader, TokenData,
    TokenQueryParams, TokenReader, TokenWriter, WatchedAddressReader, WatchedAddressWriter,
};
use uuid::Uuid;

/// In-memory implementation of all repository traits for testing.
#[derive(Default)]
pub struct InMemoryDataService {
    invoices: RwLock<HashMap<String, InvoiceData>>,
    payments: RwLock<HashMap<Uuid, PaymentData>>,
    payment_options: RwLock<HashMap<Uuid, PaymentOptionData>>,
    // Key: (address, chain_id, token_address) -> payment_option_id
    addresses: RwLock<HashMap<(String, u64, Option<String>), PaymentOptionId>>,
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
                if let Some(ref currency) = params.currency {
                    if inv.currency != *currency {
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

    fn stream_expired_pending(&self) -> BoxStream<'_, RepositoryResult<InvoiceId>> {
        let invoices = self.invoices.read().unwrap();
        let now = Utc::now();
        let ids: Vec<RepositoryResult<InvoiceId>> = invoices
            .values()
            .filter(|inv| inv.status == InvoiceStatus::Pending && inv.expires_at < now)
            .map(|inv| Ok(inv.id.clone()))
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
        chain_id: u64,
        fork_block: u64,
    ) -> RepositoryResult<u64> {
        let mut payments = self.payments.write().unwrap();
        let mut count = 0u64;
        for payment in payments.values_mut() {
            if payment.invoice_id == *invoice_id
                && payment.chain_id == chain_id
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
impl PaymentOptionReader for InMemoryDataService {
    async fn get(&self, id: &PaymentOptionId) -> RepositoryResult<Option<PaymentOptionData>> {
        let options = self.payment_options.read().unwrap();
        Ok(options.get(&id.0).cloned())
    }

    async fn get_for_invoice(&self, invoice_id: &InvoiceId) -> RepositoryResult<Vec<PaymentOptionData>> {
        let options = self.payment_options.read().unwrap();
        Ok(options
            .values()
            .filter(|po| po.invoice_id == *invoice_id)
            .cloned()
            .collect())
    }

    async fn get_by_payment_method(
        &self,
        invoice_id: &InvoiceId,
        payment_method_id: &PaymentMethodId,
    ) -> RepositoryResult<Option<PaymentOptionData>> {
        let options = self.payment_options.read().unwrap();
        Ok(options
            .values()
            .find(|po| po.invoice_id == *invoice_id && po.payment_method_id == *payment_method_id)
            .cloned())
    }

    async fn get_active_for_invoice(&self, invoice_id: &InvoiceId) -> RepositoryResult<Vec<PaymentOptionData>> {
        let options = self.payment_options.read().unwrap();
        Ok(options
            .values()
            .filter(|po| po.invoice_id == *invoice_id && po.is_active)
            .cloned()
            .collect())
    }

    async fn get_by_address(
        &self,
        address: &str,
        chain_id: u64,
        token_address: Option<&str>,
    ) -> RepositoryResult<Option<PaymentOptionData>> {
        let options = self.payment_options.read().unwrap();
        Ok(options
            .values()
            .find(|po| {
                po.payment_address == address
                    && po.chain_id == chain_id
                    && po.token_address.as_deref() == token_address
            })
            .cloned())
    }
}

#[async_trait]
impl PaymentOptionWriter for InMemoryDataService {
    async fn create(&self, option: &PaymentOptionData) -> RepositoryResult<()> {
        let mut options = self.payment_options.write().unwrap();
        options.insert(option.id.0, option.clone());
        Ok(())
    }

    async fn update(&self, option: &PaymentOptionData) -> RepositoryResult<()> {
        let mut options = self.payment_options.write().unwrap();
        if options.contains_key(&option.id.0) {
            options.insert(option.id.0, option.clone());
        }
        Ok(())
    }

    async fn deactivate(&self, id: &PaymentOptionId) -> RepositoryResult<bool> {
        let mut options = self.payment_options.write().unwrap();
        if let Some(po) = options.get_mut(&id.0) {
            if po.is_active {
                po.is_active = false;
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn deactivate_for_invoice(&self, invoice_id: &InvoiceId) -> RepositoryResult<u64> {
        let mut options = self.payment_options.write().unwrap();
        let mut count = 0u64;
        for po in options.values_mut() {
            if po.invoice_id == *invoice_id && po.is_active {
                po.is_active = false;
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
        chain_id: u64,
        token_address: Option<&str>,
    ) -> RepositoryResult<Option<InvoiceId>> {
        let addresses = self.addresses.read().unwrap();
        let options = self.payment_options.read().unwrap();

        if let Some(po_id) = addresses.get(&(address.to_string(), chain_id, token_address.map(String::from))) {
            if let Some(po) = options.get(&po_id.0) {
                return Ok(Some(po.invoice_id.clone()));
            }
        }
        Ok(None)
    }

    async fn get_payment_option_id(
        &self,
        address: &str,
        chain_id: u64,
        token_address: Option<&str>,
    ) -> RepositoryResult<Option<PaymentOptionId>> {
        let addresses = self.addresses.read().unwrap();
        Ok(addresses
            .get(&(address.to_string(), chain_id, token_address.map(String::from)))
            .cloned())
    }

    async fn get_active(&self) -> RepositoryResult<Vec<(String, PaymentOptionId, u64, Option<String>)>> {
        let addresses = self.addresses.read().unwrap();
        Ok(addresses
            .iter()
            .map(|((addr, chain_id, token_addr), po_id)| {
                (addr.clone(), po_id.clone(), *chain_id, token_addr.clone())
            })
            .collect())
    }

    async fn get_pending(&self) -> RepositoryResult<Vec<PendingWatchInfo>> {
        let addresses = self.addresses.read().unwrap();
        let options = self.payment_options.read().unwrap();

        Ok(addresses
            .iter()
            .filter_map(|((addr, chain_id, token_address), po_id)| {
                options.get(&po_id.0).map(|po| PendingWatchInfo {
                    address: addr.clone(),
                    payment_option_id: po_id.clone(),
                    invoice_id: po.invoice_id.as_str().to_string(),
                    chain_id: *chain_id,
                    expected_amount: Some(po.amount.clone()),
                    token_address: token_address.clone(),
                })
            })
            .collect())
    }

    async fn get_expired_for_cleanup(
        &self,
        _grace_period_secs: i64,
    ) -> RepositoryResult<Vec<CleanupAddressInfo>> {
        let addresses = self.addresses.read().unwrap();
        let options = self.payment_options.read().unwrap();
        let invoices = self.invoices.read().unwrap();
        let now = Utc::now();

        Ok(addresses
            .iter()
            .filter_map(|((addr, chain_id, token_address), po_id)| {
                options.get(&po_id.0).and_then(|po| {
                    invoices.get(&po.invoice_id.0).and_then(|inv| {
                        if inv.status == InvoiceStatus::Expired && inv.expires_at < now {
                            Some(CleanupAddressInfo {
                                address: addr.clone(),
                                payment_option_id: po_id.clone(),
                                invoice_id: po.invoice_id.as_str().to_string(),
                                chain_id: *chain_id,
                                token_address: token_address.clone(),
                            })
                        } else {
                            None
                        }
                    })
                })
            })
            .collect())
    }

    async fn get_paid_for_cleanup(&self) -> RepositoryResult<Vec<CleanupAddressInfo>> {
        let addresses = self.addresses.read().unwrap();
        let options = self.payment_options.read().unwrap();
        let invoices = self.invoices.read().unwrap();

        Ok(addresses
            .iter()
            .filter_map(|((addr, chain_id, token_address), po_id)| {
                options.get(&po_id.0).and_then(|po| {
                    invoices.get(&po.invoice_id.0).and_then(|inv| {
                        if inv.status == InvoiceStatus::Paid {
                            Some(CleanupAddressInfo {
                                address: addr.clone(),
                                payment_option_id: po_id.clone(),
                                invoice_id: po.invoice_id.as_str().to_string(),
                                chain_id: *chain_id,
                                token_address: token_address.clone(),
                            })
                        } else {
                            None
                        }
                    })
                })
            })
            .collect())
    }

    async fn get_cancelled_for_cleanup(&self) -> RepositoryResult<Vec<CleanupAddressInfo>> {
        let addresses = self.addresses.read().unwrap();
        let options = self.payment_options.read().unwrap();
        let invoices = self.invoices.read().unwrap();

        Ok(addresses
            .iter()
            .filter_map(|((addr, chain_id, token_address), po_id)| {
                options.get(&po_id.0).and_then(|po| {
                    invoices.get(&po.invoice_id.0).and_then(|inv| {
                        if inv.status == InvoiceStatus::Cancelled {
                            Some(CleanupAddressInfo {
                                address: addr.clone(),
                                payment_option_id: po_id.clone(),
                                invoice_id: po.invoice_id.as_str().to_string(),
                                chain_id: *chain_id,
                                token_address: token_address.clone(),
                            })
                        } else {
                            None
                        }
                    })
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
        payment_option_id: &PaymentOptionId,
        chain_id: u64,
        token_address: Option<&str>,
    ) -> RepositoryResult<()> {
        let mut addresses = self.addresses.write().unwrap();
        addresses.insert(
            (address.to_string(), chain_id, token_address.map(String::from)),
            payment_option_id.clone(),
        );
        Ok(())
    }

    async fn mark_notified(
        &self,
        _address: &str,
        _chain_id: u64,
        _token_address: Option<&str>,
    ) -> RepositoryResult<()> {
        // No-op for in-memory testing
        Ok(())
    }

    async fn deactivate(
        &self,
        address: &str,
        chain_id: u64,
        token_address: Option<&str>,
    ) -> RepositoryResult<bool> {
        let mut addresses = self.addresses.write().unwrap();
        let key = (address.to_string(), chain_id, token_address.map(String::from));
        if addresses.remove(&key).is_some() {
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn deactivate_for_payment_option(
        &self,
        payment_option_id: &PaymentOptionId,
    ) -> RepositoryResult<u64> {
        let mut addresses = self.addresses.write().unwrap();
        let keys_to_remove: Vec<_> = addresses
            .iter()
            .filter(|(_, po_id)| **po_id == *payment_option_id)
            .map(|(k, _)| k.clone())
            .collect();
        let count = keys_to_remove.len() as u64;
        for key in keys_to_remove {
            addresses.remove(&key);
        }
        Ok(count)
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

/// Create a test invoice (network-agnostic).
pub fn create_test_invoice() -> InvoiceData {
    InvoiceData {
        id: InvoiceId::new(),
        store_id: StoreId::new(),
        currency: "USD".to_string(),
        status: InvoiceStatus::Pending,
        amount: "100.00".to_string(), // $100 USD
        amount_received: "0".to_string(),
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        metadata: None,
        extra: None,
    }
}

/// Create a test payment option for an invoice.
pub fn create_test_payment_option(invoice_id: &InvoiceId) -> PaymentOptionData {
    PaymentOptionData {
        id: PaymentOptionId(Uuid::new_v4()),
        invoice_id: invoice_id.clone(),
        payment_method_id: PaymentMethodId::new("ETH", 1),
        chain_id: 1,
        asset_symbol: "ETH".to_string(),
        token_address: None,
        decimals: 18,
        payment_address: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
        amount: "50000000000000000".to_string(), // ~0.05 ETH worth $100 at hypothetical rate
        rate: Some("2000.00".to_string()),
        rate_at: Some(Utc::now()),
        is_active: true,
        created_at: Utc::now(),
    }
}

/// Create a test payment.
pub fn create_test_payment(invoice_id: &InvoiceId, payment_option_id: Option<&PaymentOptionId>) -> PaymentData {
    PaymentData {
        id: Uuid::new_v4(),
        invoice_id: invoice_id.clone(),
        payment_option_id: payment_option_id.map(|po| po.0),
        chain_id: 1, // Ethereum mainnet
        asset_type: types::AssetType::Native,
        amount: "50000000000000000".to_string(),
        asset_symbol: "ETH".to_string(),
        token_address: None,
        tx_hash: "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890".to_string(),
        block_number: Some(12345678),
        detected_at: Utc::now(),
        confirmed_at: None,
        from_address: Some("0xabcdef1234567890abcdef1234567890abcdef12".to_string()),
        reorged: false,
        extra: None,
        credited_amount: Some("0.05".to_string()), // 0.05 ETH
        rate_used: None,
        rate_applied_at: None,
    }
}
