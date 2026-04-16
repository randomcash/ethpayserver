//! Transaction building and signing for refunds and payouts.
//!
//! Provides infrastructure for constructing, signing, and broadcasting
//! outbound transactions from derived payment addresses.

use alloy::consensus::TxEnvelope;
use alloy::network::{EthereumWallet, TransactionBuilder};
use alloy::primitives::{Address, U256};
use alloy::providers::Provider;
use alloy::rpc::types::TransactionRequest;
use alloy::signers::local::PrivateKeySigner;

use crate::error::{EvmError, EvmResult};
use crate::provider::EvmProvider;
use crate::tokens::IERC20;
use crate::wallet::HdWallet;

/// Gas limit for a native ETH transfer (21,000 is the standard).
const NATIVE_TRANSFER_GAS: u64 = 21_000;

/// Gas limit safety multiplier numerator (110%).
const GAS_MULTIPLIER_NUM: u64 = 110;
/// Gas limit safety multiplier denominator.
const GAS_MULTIPLIER_DEN: u64 = 100;

/// Result of a signed and broadcast transaction.
#[derive(Debug, Clone)]
pub struct TransactionResult {
    /// Transaction hash.
    pub tx_hash: String,
    /// Gas fee paid (estimated, in wei).
    pub fee_wei: U256,
}

/// Build and sign a native token (ETH/MATIC/etc.) transfer.
///
/// # Arguments
///
/// * `wallet` - HD wallet containing the master key
/// * `address_index` - BIP-44 index of the derived address to send from
/// * `provider` - RPC provider for the target chain
/// * `to` - Recipient address
/// * `amount` - Amount to send in wei
///
/// # Security
///
/// The private key is derived in memory, used for signing, and then dropped.
/// It is never persisted or transmitted.
pub async fn send_native_transfer(
    wallet: &HdWallet,
    address_index: u32,
    provider: &EvmProvider,
    to: Address,
    amount: U256,
) -> EvmResult<TransactionResult> {
    let private_key_bytes = wallet.derive_private_key(address_index)?;
    let signer = PrivateKeySigner::from_slice(&private_key_bytes)
        .map_err(|e| EvmError::Signing(e.to_string()))?;
    let from = signer.address();

    // Check balance
    let balance = provider.get_native_balance(from).await?;
    let gas_price = provider
        .inner()
        .get_gas_price()
        .await
        .map_err(|e| EvmError::Rpc(e.to_string()))?;

    let gas_cost = U256::from(NATIVE_TRANSFER_GAS) * U256::from(gas_price);
    let total_needed = amount + gas_cost;

    if balance < total_needed {
        return Err(EvmError::InsufficientBalance(format!(
            "need {} wei (amount {} + gas {}), have {} wei",
            total_needed, amount, gas_cost, balance
        )));
    }

    // Build transaction
    let nonce = provider.get_transaction_count(from).await?;
    let chain_id = provider.chain_config().chain_id;

    let tx = TransactionRequest::default()
        .with_from(from)
        .with_to(to)
        .with_value(amount)
        .with_nonce(nonce)
        .with_gas_limit(NATIVE_TRANSFER_GAS as u128)
        .with_gas_price(gas_price)
        .with_chain_id(chain_id);

    // Sign and send
    let eth_wallet = EthereumWallet::from(signer);
    let signed_tx = tx
        .build(&eth_wallet)
        .await
        .map_err(|e| EvmError::Signing(e.to_string()))?;

    let tx_hash = send_raw_transaction(provider, signed_tx).await?;

    Ok(TransactionResult {
        tx_hash,
        fee_wei: gas_cost,
    })
}

/// Build and sign an ERC20 token transfer.
///
/// # Arguments
///
/// * `wallet` - HD wallet containing the master key
/// * `address_index` - BIP-44 index of the derived address to send from
/// * `provider` - RPC provider for the target chain
/// * `token_address` - ERC20 contract address
/// * `to` - Recipient address
/// * `amount` - Amount to send in the token's smallest unit
pub async fn send_erc20_transfer(
    wallet: &HdWallet,
    address_index: u32,
    provider: &EvmProvider,
    token_address: Address,
    to: Address,
    amount: U256,
) -> EvmResult<TransactionResult> {
    let private_key_bytes = wallet.derive_private_key(address_index)?;
    let signer = PrivateKeySigner::from_slice(&private_key_bytes)
        .map_err(|e| EvmError::Signing(e.to_string()))?;
    let from = signer.address();

    // Encode the ERC20 transfer call
    let call = IERC20::transferCall { to, amount };
    let calldata = alloy::sol_types::SolCall::abi_encode(&call);

    // Estimate gas for the ERC20 transfer
    let estimate_tx = TransactionRequest::default()
        .with_from(from)
        .with_to(token_address)
        .with_input(calldata.clone());

    let estimated_gas = provider
        .inner()
        .estimate_gas(&estimate_tx)
        .await
        .map_err(|e| EvmError::Rpc(format!("gas estimation failed: {}", e)))?;

    // Apply safety multiplier
    let gas_limit = estimated_gas * GAS_MULTIPLIER_NUM / GAS_MULTIPLIER_DEN;

    let gas_price = provider
        .inner()
        .get_gas_price()
        .await
        .map_err(|e| EvmError::Rpc(e.to_string()))?;

    // Check native balance covers gas
    let gas_cost = U256::from(gas_limit) * U256::from(gas_price);
    let native_balance = provider.get_native_balance(from).await?;
    if native_balance < gas_cost {
        return Err(EvmError::InsufficientBalance(format!(
            "need {} wei for gas, have {} wei",
            gas_cost, native_balance
        )));
    }

    // Check token balance
    let token_balance =
        crate::tokens::get_token_balance(provider.inner(), &token_address.to_string(), from)
            .await?;
    if token_balance < amount {
        return Err(EvmError::InsufficientBalance(format!(
            "need {} token units, have {}",
            amount, token_balance
        )));
    }

    // Build transaction
    let nonce = provider.get_transaction_count(from).await?;
    let chain_id = provider.chain_config().chain_id;

    let tx = TransactionRequest::default()
        .with_from(from)
        .with_to(token_address)
        .with_input(calldata)
        .with_nonce(nonce)
        .with_gas_limit(gas_limit as u128)
        .with_gas_price(gas_price)
        .with_chain_id(chain_id);

    // Sign and send
    let eth_wallet = EthereumWallet::from(signer);
    let signed_tx = tx
        .build(&eth_wallet)
        .await
        .map_err(|e| EvmError::Signing(e.to_string()))?;

    let tx_hash = send_raw_transaction(provider, signed_tx).await?;

    Ok(TransactionResult {
        tx_hash,
        fee_wei: gas_cost,
    })
}

/// Send a signed transaction via the provider.
async fn send_raw_transaction(provider: &EvmProvider, signed_tx: TxEnvelope) -> EvmResult<String> {
    let pending = provider
        .inner()
        .send_tx_envelope(signed_tx)
        .await
        .map_err(|e| EvmError::Transaction(format!("broadcast failed: {}", e)))?;

    Ok(format!("{:#x}", pending.tx_hash()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gas_multiplier() {
        let estimated: u64 = 50_000;
        let adjusted = estimated * GAS_MULTIPLIER_NUM / GAS_MULTIPLIER_DEN;
        assert_eq!(adjusted, 55_000);
    }

    #[test]
    fn test_native_transfer_gas_constant() {
        assert_eq!(NATIVE_TRANSFER_GAS, 21_000);
    }
}
