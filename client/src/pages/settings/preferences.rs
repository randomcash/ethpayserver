//! Preferences settings tab.

use leptos::prelude::*;

/// Preferences tab.
#[component]
pub fn PreferencesTab() -> impl IntoView {
    let (theme, set_theme) = signal("system".to_string());
    let (currency, set_currency) = signal("USD".to_string());
    let (timezone, set_timezone) = signal("UTC".to_string());
    let (date_format, set_date_format) = signal("mdy".to_string());

    view! {
        <div class="settings-tab-preferences">
            <div class="detail-card">
                <div class="detail-card-header">
                    <h3>"Appearance"</h3>
                </div>
                <div class="detail-card-body">
                    <div class="form-group">
                        <label class="form-label">"Theme"</label>
                        <select
                            class="form-select"
                            prop:value=move || theme.get()
                            on:change=move |ev| set_theme.set(event_target_value(&ev))
                        >
                            <option value="system">"System default"</option>
                            <option value="light">"Light"</option>
                            <option value="dark">"Dark"</option>
                        </select>
                    </div>
                </div>
            </div>

            <div class="detail-card">
                <div class="detail-card-header">
                    <h3>"Regional Settings"</h3>
                </div>
                <div class="detail-card-body">
                    <div class="form-group">
                        <label class="form-label">"Default Currency"</label>
                        <select
                            class="form-select"
                            prop:value=move || currency.get()
                            on:change=move |ev| set_currency.set(event_target_value(&ev))
                        >
                            <option value="USD">"USD - US Dollar"</option>
                            <option value="EUR">"EUR - Euro"</option>
                            <option value="GBP">"GBP - British Pound"</option>
                            <option value="JPY">"JPY - Japanese Yen"</option>
                            <option value="BTC">"BTC - Bitcoin"</option>
                            <option value="ETH">"ETH - Ethereum"</option>
                        </select>
                        <p class="form-help">"Used for displaying amounts in your preferred currency"</p>
                    </div>

                    <div class="form-group">
                        <label class="form-label">"Timezone"</label>
                        <select
                            class="form-select"
                            prop:value=move || timezone.get()
                            on:change=move |ev| set_timezone.set(event_target_value(&ev))
                        >
                            <option value="UTC">"UTC"</option>
                            <option value="America/New_York">"Eastern Time (US)"</option>
                            <option value="America/Los_Angeles">"Pacific Time (US)"</option>
                            <option value="Europe/London">"London"</option>
                            <option value="Europe/Paris">"Paris"</option>
                            <option value="Asia/Tokyo">"Tokyo"</option>
                            <option value="Asia/Shanghai">"Shanghai"</option>
                        </select>
                    </div>

                    <div class="form-group">
                        <label class="form-label">"Date Format"</label>
                        <select
                            class="form-select"
                            prop:value=move || date_format.get()
                            on:change=move |ev| set_date_format.set(event_target_value(&ev))
                        >
                            <option value="mdy">"MM/DD/YYYY"</option>
                            <option value="dmy">"DD/MM/YYYY"</option>
                            <option value="ymd">"YYYY-MM-DD"</option>
                        </select>
                    </div>

                    <div class="form-actions">
                        <button class="btn btn-primary btn-sm">"Save preferences"</button>
                    </div>
                </div>
            </div>
        </div>
    }
}
