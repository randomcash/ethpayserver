//! Chain connection status component.

use leptos::prelude::*;
use types::Network;
use ui_kit::components::Spinner;
use ui_kit::components::crypto::NetworkBadge;

/// Chain connection status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionStatus {
    Connected,
    Connecting,
    Disconnected,
    Error,
}

impl ConnectionStatus {
    fn class(&self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Connecting => "connecting",
            Self::Disconnected => "disconnected",
            Self::Error => "error",
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Connected => "Connected",
            Self::Connecting => "Connecting...",
            Self::Disconnected => "Disconnected",
            Self::Error => "Error",
        }
    }
}

/// Chain status info.
#[derive(Clone, Debug)]
pub struct ChainInfo {
    pub network: Network,
    pub chain_id: u64,
    pub status: ConnectionStatus,
    pub block_number: Option<u64>,
    pub latency_ms: Option<u32>,
}

/// Chain status component.
#[component]
pub fn ChainStatus(
    /// Chain information.
    info: ChainInfo,
    /// Whether to show detailed info.
    #[prop(default = false)]
    detailed: bool,
) -> impl IntoView {
    let status_class = format!("evm-chain-status {}", info.status.class());

    view! {
        <div class=status_class>
            <div class="evm-chain-status-header">
                <NetworkBadge network=info.network />
                <span class="evm-chain-status-indicator">
                    {match info.status {
                        ConnectionStatus::Connecting => view! { <Spinner size="sm" /> }.into_any(),
                        _ => view! { <span class="evm-chain-status-dot"></span> }.into_any(),
                    }}
                    <span class="evm-chain-status-label">{info.status.label()}</span>
                </span>
            </div>

            {detailed.then(|| view! {
                <div class="evm-chain-status-details">
                    {info.block_number.map(|block| view! {
                        <div class="evm-chain-detail">
                            <span class="evm-chain-detail-label">"Block"</span>
                            <span class="evm-chain-detail-value">{format!("#{}", block)}</span>
                        </div>
                    })}
                    {info.latency_ms.map(|latency| view! {
                        <div class="evm-chain-detail">
                            <span class="evm-chain-detail-label">"Latency"</span>
                            <span class="evm-chain-detail-value">{format!("{}ms", latency)}</span>
                        </div>
                    })}
                    <div class="evm-chain-detail">
                        <span class="evm-chain-detail-label">"Chain ID"</span>
                        <span class="evm-chain-detail-value">{info.chain_id}</span>
                    </div>
                </div>
            })}
        </div>
    }
}

/// Chain status list component.
#[allow(dead_code)]
#[component]
pub fn ChainStatusList(
    /// List of chain info.
    chains: Vec<ChainInfo>,
) -> impl IntoView {
    view! {
        <div class="evm-chain-status-list">
            {chains.into_iter().map(|chain| {
                view! { <ChainStatus info=chain detailed=true /> }
            }).collect_view()}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_status_class() {
        assert_eq!(ConnectionStatus::Connected.class(), "connected");
        assert_eq!(ConnectionStatus::Connecting.class(), "connecting");
        assert_eq!(ConnectionStatus::Disconnected.class(), "disconnected");
        assert_eq!(ConnectionStatus::Error.class(), "error");
    }

    #[test]
    fn test_connection_status_label() {
        assert_eq!(ConnectionStatus::Connected.label(), "Connected");
        assert_eq!(ConnectionStatus::Connecting.label(), "Connecting...");
        assert_eq!(ConnectionStatus::Disconnected.label(), "Disconnected");
        assert_eq!(ConnectionStatus::Error.label(), "Error");
    }

    #[test]
    fn test_chain_info_creation() {
        let info = ChainInfo {
            network: Network::Ethereum,
            chain_id: 1,
            status: ConnectionStatus::Connected,
            block_number: Some(18000000),
            latency_ms: Some(50),
        };

        assert_eq!(info.chain_id, 1);
        assert_eq!(info.status, ConnectionStatus::Connected);
        assert!(info.block_number.is_some());
    }

    #[test]
    fn test_connection_status_equality() {
        assert_eq!(ConnectionStatus::Connected, ConnectionStatus::Connected);
        assert_ne!(ConnectionStatus::Connected, ConnectionStatus::Disconnected);
    }
}

/// Chain status styles.
#[allow(dead_code)]
pub const CHAIN_STATUS_STYLES: &str = r#"
.evm-chain-status {
    padding: var(--ps-spacing-md);
    background-color: var(--ps-surface);
    border: 1px solid var(--ps-border);
    border-radius: var(--ps-radius-md);
}

.evm-chain-status-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
}

.evm-chain-status-indicator {
    display: flex;
    align-items: center;
    gap: var(--ps-spacing-xs);
    font-size: var(--ps-font-sm);
}

.evm-chain-status-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
}

.evm-chain-status.connected .evm-chain-status-dot {
    background-color: var(--ps-success);
}

.evm-chain-status.connecting .evm-chain-status-dot {
    background-color: var(--ps-warning);
}

.evm-chain-status.disconnected .evm-chain-status-dot {
    background-color: var(--ps-text-muted);
}

.evm-chain-status.error .evm-chain-status-dot {
    background-color: var(--ps-error);
}

.evm-chain-status-label {
    color: var(--ps-text-muted);
}

.evm-chain-status.connected .evm-chain-status-label {
    color: var(--ps-success);
}

.evm-chain-status.error .evm-chain-status-label {
    color: var(--ps-error);
}

.evm-chain-status-details {
    display: flex;
    gap: var(--ps-spacing-lg);
    margin-top: var(--ps-spacing-md);
    padding-top: var(--ps-spacing-md);
    border-top: 1px solid var(--ps-border);
}

.evm-chain-detail {
    display: flex;
    flex-direction: column;
    gap: var(--ps-spacing-xs);
}

.evm-chain-detail-label {
    font-size: var(--ps-font-sm);
    color: var(--ps-text-muted);
}

.evm-chain-detail-value {
    font-family: monospace;
    font-weight: 500;
}

.evm-chain-status-list {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: var(--ps-spacing-md);
}
"#;
