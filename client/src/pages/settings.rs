//! Settings page.

use leptos::prelude::*;
use ui_kit::components::{
    Button, ButtonVariant, CardWithHeader, Checkbox, Grid, Input, PageHeader, Stack,
};

/// Settings page.
#[component]
pub fn SettingsPage() -> impl IntoView {
    // Settings state
    let api_url = RwSignal::new("http://localhost:5000".to_string());
    let webhook_url = RwSignal::new(String::new());
    let auto_confirm = RwSignal::new(true);
    let require_confirmations = RwSignal::new("12".to_string());

    view! {
        <div class="evm-page">
            <PageHeader
                title="Settings"
                description="Configure your EVM payment server"
            />

            <Stack gap="lg">
                <Grid cols=2 gap="lg">
                    // API Configuration
                    <CardWithHeader title="API Configuration">
                        <Stack gap="md">
                            <Input
                                value=api_url
                                label="API URL"
                                placeholder="http://localhost:5000"
                            />
                            <Input
                                value=webhook_url
                                label="Webhook URL"
                                placeholder="https://your-site.com/webhook"
                            />
                        </Stack>
                    </CardWithHeader>

                    // Payment Settings
                    <CardWithHeader title="Payment Settings">
                        <Stack gap="md">
                            <Checkbox
                                checked=auto_confirm
                                label="Auto-confirm payments"
                            />
                            <Input
                                value=require_confirmations
                                label="Required Confirmations"
                                input_type="number"
                            />
                        </Stack>
                    </CardWithHeader>
                </Grid>

                // Network Configuration
                <CardWithHeader title="Default Networks">
                    <Stack gap="sm">
                        <p class="evm-settings-hint">
                            "Select which networks are enabled by default for new stores"
                        </p>
                        <NetworkCheckboxList />
                    </Stack>
                </CardWithHeader>

                // Save button
                <div class="evm-page-actions">
                    <Button variant=ButtonVariant::Primary>
                        "Save Settings"
                    </Button>
                </div>
            </Stack>
        </div>
    }
}

#[component]
fn NetworkCheckboxList() -> impl IntoView {
    use ui_kit::components::crypto::NetworkBadge;

    let networks = vec![
        (types::Network::Ethereum, true),
        (types::Network::Polygon, true),
        (types::Network::Arbitrum, true),
        (types::Network::Optimism, false),
        (types::Network::Base, true),
        (types::Network::Avalanche, false),
        (types::Network::BinanceSmartChain, false),
        (types::Network::ZkSync, false),
        (types::Network::Linea, false),
        (types::Network::Scroll, false),
    ];

    view! {
        <div class="evm-network-checkbox-list">
            {networks.into_iter().map(|(network, default_enabled)| {
                let enabled = RwSignal::new(default_enabled);
                view! {
                    <div class="evm-network-checkbox-item">
                        <Checkbox checked=enabled />
                        <NetworkBadge network=network />
                    </div>
                }
            }).collect_view()}
        </div>
    }
}

/// Settings page styles.
pub const SETTINGS_STYLES: &str = r#"
.evm-settings-hint {
    font-size: var(--ps-font-sm);
    color: var(--ps-text-muted);
    margin: 0 0 var(--ps-spacing-md) 0;
}

.evm-network-checkbox-list {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: var(--ps-spacing-sm);
}

.evm-network-checkbox-item {
    display: flex;
    align-items: center;
    gap: var(--ps-spacing-sm);
    padding: var(--ps-spacing-sm);
    background-color: var(--ps-background);
    border-radius: var(--ps-radius-md);
}
"#;
