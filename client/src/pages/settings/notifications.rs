//! Notifications settings tab.

use leptos::prelude::*;

/// Notifications tab.
#[component]
pub fn NotificationsTab() -> impl IntoView {
    let (email_payments, set_email_payments) = signal(true);
    let (email_invoices, set_email_invoices) = signal(true);
    let (email_security, set_email_security) = signal(true);
    let (email_marketing, set_email_marketing) = signal(false);

    view! {
        <div class="settings-tab-notifications">
            <div class="detail-card">
                <div class="detail-card-header">
                    <h3>"Email Notifications"</h3>
                </div>
                <div class="detail-card-body">
                    <div class="notification-options">
                        <div class="notification-option">
                            <div class="notification-option-info">
                                <span class="notification-option-title">"Payment notifications"</span>
                                <span class="notification-option-desc">"Get notified when payments are received or confirmed"</span>
                            </div>
                            <label class="toggle">
                                <input
                                    type="checkbox"
                                    prop:checked=move || email_payments.get()
                                    on:change=move |ev| set_email_payments.set(event_target_checked(&ev))
                                />
                                <span class="toggle-slider"></span>
                            </label>
                        </div>

                        <div class="notification-option">
                            <div class="notification-option-info">
                                <span class="notification-option-title">"Invoice updates"</span>
                                <span class="notification-option-desc">"Notifications for invoice status changes"</span>
                            </div>
                            <label class="toggle">
                                <input
                                    type="checkbox"
                                    prop:checked=move || email_invoices.get()
                                    on:change=move |ev| set_email_invoices.set(event_target_checked(&ev))
                                />
                                <span class="toggle-slider"></span>
                            </label>
                        </div>

                        <div class="notification-option">
                            <div class="notification-option-info">
                                <span class="notification-option-title">"Security alerts"</span>
                                <span class="notification-option-desc">"Important security notifications and login alerts"</span>
                            </div>
                            <label class="toggle">
                                <input
                                    type="checkbox"
                                    prop:checked=move || email_security.get()
                                    on:change=move |ev| set_email_security.set(event_target_checked(&ev))
                                />
                                <span class="toggle-slider"></span>
                            </label>
                        </div>

                        <div class="notification-option">
                            <div class="notification-option-info">
                                <span class="notification-option-title">"Product updates"</span>
                                <span class="notification-option-desc">"News about new features and improvements"</span>
                            </div>
                            <label class="toggle">
                                <input
                                    type="checkbox"
                                    prop:checked=move || email_marketing.get()
                                    on:change=move |ev| set_email_marketing.set(event_target_checked(&ev))
                                />
                                <span class="toggle-slider"></span>
                            </label>
                        </div>
                    </div>

                    <div class="form-actions">
                        <button class="btn btn-primary btn-sm">"Save notification settings"</button>
                    </div>
                </div>
            </div>
        </div>
    }
}
