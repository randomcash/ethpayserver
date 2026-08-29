//! In-memory `StorePaymentMethod` repositories for `InMemoryDataService`.
//!
//! Kept in its own file so the main `test_utils` module stays readable.

use async_trait::async_trait;
use chrono::Utc;
use types::{
    RepositoryError, RepositoryResult, StorePaymentMethod, StorePaymentMethodReader,
    StorePaymentMethodWriter,
};
use uuid::Uuid;

use super::InMemoryDataService;

impl InMemoryDataService {
    /// Register an enabled payment method for a store (for testing).
    ///
    /// Returns the generated payment method ID. Methods are returned by the
    /// reader in insertion order.
    pub fn add_payment_method(
        &self,
        store_id: Uuid,
        chain_id: u64,
        asset_symbol: &str,
        decimals: u8,
        xpub: &str,
    ) -> Uuid {
        let id = Uuid::new_v4();
        self.payment_methods
            .write()
            .unwrap()
            .push(StorePaymentMethod {
                id,
                store_id,
                chain_id,
                token_address: None,
                asset_symbol: asset_symbol.to_string(),
                decimals,
                xpub: xpub.to_string(),
                derivation_index: 0,
                enabled: true,
                created_at: Utc::now(),
            });
        id
    }

    /// Read back a payment method's current derivation index (for testing).
    pub fn derivation_index(&self, id: Uuid) -> Option<i32> {
        self.payment_methods
            .read()
            .unwrap()
            .iter()
            .find(|pm| pm.id == id)
            .map(|pm| pm.derivation_index)
    }
}

#[async_trait]
impl StorePaymentMethodReader for InMemoryDataService {
    async fn get_payment_methods(
        &self,
        store_id: Uuid,
    ) -> RepositoryResult<Vec<StorePaymentMethod>> {
        Ok(self
            .payment_methods
            .read()
            .unwrap()
            .iter()
            .filter(|pm| pm.store_id == store_id)
            .cloned()
            .collect())
    }

    async fn get_enabled_payment_methods(
        &self,
        store_id: Uuid,
    ) -> RepositoryResult<Vec<StorePaymentMethod>> {
        Ok(self
            .payment_methods
            .read()
            .unwrap()
            .iter()
            .filter(|pm| pm.store_id == store_id && pm.enabled)
            .cloned()
            .collect())
    }

    async fn get_payment_method(&self, id: Uuid) -> RepositoryResult<Option<StorePaymentMethod>> {
        Ok(self
            .payment_methods
            .read()
            .unwrap()
            .iter()
            .find(|pm| pm.id == id)
            .cloned())
    }

    async fn get_payment_method_by_chain(
        &self,
        store_id: Uuid,
        chain_id: u64,
        token_address: Option<&str>,
    ) -> RepositoryResult<Option<StorePaymentMethod>> {
        Ok(self
            .payment_methods
            .read()
            .unwrap()
            .iter()
            .find(|pm| {
                pm.store_id == store_id
                    && pm.chain_id == chain_id
                    && pm.token_address.as_deref() == token_address
            })
            .cloned())
    }

    async fn find_by_asset_symbol(
        &self,
        store_id: Uuid,
        asset_symbol: &str,
    ) -> RepositoryResult<Vec<StorePaymentMethod>> {
        Ok(self
            .payment_methods
            .read()
            .unwrap()
            .iter()
            .filter(|pm| pm.store_id == store_id && pm.asset_symbol == asset_symbol)
            .cloned()
            .collect())
    }
}

#[async_trait]
impl StorePaymentMethodWriter for InMemoryDataService {
    async fn create_payment_method(
        &self,
        store_id: Uuid,
        chain_id: u64,
        token_address: Option<&str>,
        asset_symbol: &str,
        decimals: u8,
        xpub: &str,
    ) -> RepositoryResult<StorePaymentMethod> {
        let method = StorePaymentMethod {
            id: Uuid::new_v4(),
            store_id,
            chain_id,
            token_address: token_address.map(str::to_string),
            asset_symbol: asset_symbol.to_string(),
            decimals,
            xpub: xpub.to_string(),
            derivation_index: 0,
            enabled: true,
            created_at: Utc::now(),
        };
        self.payment_methods.write().unwrap().push(method.clone());
        Ok(method)
    }

    async fn update_payment_method(
        &self,
        id: Uuid,
        enabled: Option<bool>,
        xpub: Option<&str>,
    ) -> RepositoryResult<StorePaymentMethod> {
        let mut methods = self.payment_methods.write().unwrap();
        let method = methods
            .iter_mut()
            .find(|pm| pm.id == id)
            .ok_or_else(|| RepositoryError::NotFound(format!("payment method {id}")))?;
        if let Some(enabled) = enabled {
            method.enabled = enabled;
        }
        if let Some(xpub) = xpub {
            method.xpub = xpub.to_string();
        }
        Ok(method.clone())
    }

    async fn delete_payment_method(&self, id: Uuid) -> RepositoryResult<()> {
        self.payment_methods
            .write()
            .unwrap()
            .retain(|pm| pm.id != id);
        Ok(())
    }

    async fn next_derivation_index(&self, id: Uuid) -> RepositoryResult<i32> {
        let mut methods = self.payment_methods.write().unwrap();
        let method = methods
            .iter_mut()
            .find(|pm| pm.id == id)
            .ok_or_else(|| RepositoryError::NotFound(format!("payment method {id}")))?;
        let current = method.derivation_index;
        method.derivation_index += 1;
        Ok(current)
    }
}
