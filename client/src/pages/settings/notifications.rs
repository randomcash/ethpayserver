//! Notifications settings tab: which events reach which channel, per store.
//!
//! This tab used to offer four account-wide toggles — "Payment notifications",
//! "Invoice updates", "Security alerts", "Product updates" — bound to nothing
//! and saved by nothing. The vocabulary was the deeper problem: the server has
//! no account-wide preference store and no notion of a "security alert", so
//! there was nothing to wire those toggles to. What it has is per-store
//! `store_settings.notification_prefs`, keyed by event, with a flag per
//! channel. This tab is drawn against that instead (RCS-228).
//!
//! "Product updates" has no successor here on purpose. It is not an event the
//! server emits, it is a marketing mailing list — which needs a list, consent
//! records and an unsubscribe path before a toggle over it means anything.
//! Carrying it across as a checkbox that saves nothing, or one that saves a
//! key nothing reads, is the same defect this tab is being rebuilt to fix.

use leptos::prelude::*;
use serde_json::{Value, json};

use crate::api::{EvmApiClient, UpdateStoreSettingsRequest};
use crate::app::StoreContext;
use crate::components::NoStoreSelected;

/// What the email channel does for an event.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum EmailChannel {
    /// The server sends no mail for this event, so there is no cell to fill.
    Unused,
    /// The customer payment receipt, gated by [`CUSTOMER_RECEIPTS_KEY`].
    CustomerReceipt,
}

/// One row of the matrix.
struct NotificationEvent {
    /// The `notification_prefs` key, and what the webhook dispatcher looks up.
    key: &'static str,
    label: &'static str,
    description: &'static str,
    email: EmailChannel,
}

/// The events, in the order an invoice moves through them.
///
/// This list must stay equal to `VALID_NOTIFICATION_EVENTS` in
/// `server/src/api/stores/settings.rs`: a key invented here is a 400 on save,
/// and one left out is a webhook the merchant cannot switch off.
const NOTIFICATION_EVENTS: [NotificationEvent; 5] = [
    NotificationEvent {
        key: "payment_detected",
        label: "Payment detected",
        description: "A payment has arrived on-chain but is not yet confirmed",
        email: EmailChannel::Unused,
    },
    NotificationEvent {
        key: "payment_confirmed",
        label: "Payment confirmed",
        description: "A payment reached the required confirmations",
        email: EmailChannel::CustomerReceipt,
    },
    NotificationEvent {
        key: "invoice_expired",
        label: "Invoice expired",
        description: "An invoice passed its expiry without being paid",
        email: EmailChannel::Unused,
    },
    NotificationEvent {
        key: "invoice_cancelled",
        label: "Invoice cancelled",
        description: "An invoice was cancelled before it was paid",
        email: EmailChannel::Unused,
    },
    NotificationEvent {
        key: "late_paid",
        label: "Paid late",
        description: "A payment arrived after the invoice had expired",
        email: EmailChannel::Unused,
    },
];

/// The prefs key gating the customer receipt email.
const CUSTOMER_RECEIPTS_KEY: &str = "customer_receipts_enabled";

/// Whether the webhook for `event` is on, per the store's prefs blob.
///
/// Absent means on, matching the dispatcher: `webhook_dispatch.rs` suppresses
/// only on an explicit `false`, so a store that has never saved this tab still
/// gets every webhook. Reading a missing key as "off" here would draw the
/// matrix as the exact inverse of what the server does.
fn webhook_enabled(prefs: &Value, event: &str) -> bool {
    prefs.get(event).and_then(|e| e.get("webhook")) != Some(&Value::Bool(false))
}

/// Whether customer receipt emails are on for the store.
///
/// Same "absent means on" rule, from `confirmation_handler.rs`.
fn customer_receipts_enabled(prefs: &Value) -> bool {
    prefs.get(CUSTOMER_RECEIPTS_KEY) != Some(&Value::Bool(false))
}

/// The whole matrix read out of a prefs blob, in [`NOTIFICATION_EVENTS`] order.
fn read_matrix(prefs: &Value) -> [bool; NOTIFICATION_EVENTS.len()] {
    let mut cells = [true; NOTIFICATION_EVENTS.len()];
    for (cell, event) in cells.iter_mut().zip(NOTIFICATION_EVENTS.iter()) {
        *cell = webhook_enabled(prefs, event.key);
    }
    cells
}

/// The `notification_prefs` blob to PATCH for a given matrix.
///
/// Writes all five keys and nothing else. Not a merge onto what was loaded,
/// deliberately: the PATCH validator rejects any top-level key outside
/// `VALID_NOTIFICATION_EVENTS`, so carrying `customer_receipts_enabled`
/// through would turn every save into a 400. The cost is that saving here
/// drops that key — see the note rendered under the matrix (RCS-228).
fn write_matrix(cells: &[bool; NOTIFICATION_EVENTS.len()]) -> Value {
    let mut obj = serde_json::Map::new();
    for (cell, event) in cells.iter().zip(NOTIFICATION_EVENTS.iter()) {
        obj.insert(event.key.to_string(), json!({ "webhook": *cell }));
    }
    Value::Object(obj)
}

/// Notifications tab.
#[component]
pub fn NotificationsTab() -> impl IntoView {
    let api = use_context::<Signal<EvmApiClient>>().expect("EvmApiClient must be provided");
    let store_ctx = use_context::<StoreContext>().expect("StoreContext must be provided");

    let selected_store_id = store_ctx.selected_store_id;
    let store_status = store_ctx.stores_status;
    let has_stores = Signal::derive({
        let ctx = store_ctx.clone();
        move || !ctx.stores.get().is_empty()
    });
    let retry_stores = Callback::new({
        let ctx = store_ctx.clone();
        move |()| ctx.refetch_stores()
    });

    let (refresh, set_refresh) = signal(0u32);
    let settings = LocalResource::new(move || {
        let api = api.get();
        let store_id = selected_store_id.get();
        refresh.get();
        async move {
            match store_id {
                Some(id) => Some(api.get_store_settings(&id).await),
                None => None,
            }
        }
    });

    // The matrix: one webhook flag per event, parallel to NOTIFICATION_EVENTS.
    let cells = RwSignal::new([true; NOTIFICATION_EVENTS.len()]);
    // Receipts are shown, not edited — the key cannot be PATCHed yet.
    let receipts = RwSignal::new(true);
    // Which store the matrix currently holds, not merely "something loaded":
    // the sidebar can switch stores under this tab, and a plain `loaded` flag
    // would leave the previous store's answers on screen — and then save them
    // onto the new store.
    let (loaded_store, set_loaded_store) = signal(Option::<String>::None);
    let (saving, set_saving) = signal(false);
    let (error_msg, set_error_msg) = signal(Option::<String>::None);
    let (saved, set_saved) = signal(false);

    // Saveable only while the matrix on screen belongs to the store the save
    // would go to. Between picking another store and its settings landing,
    // those are two different stores.
    let ready = Signal::derive(move || {
        let loaded = loaded_store.get();
        loaded.is_some() && loaded == selected_store_id.get()
    });

    let on_save = move |_| {
        let Some(store_id) = selected_store_id.get() else {
            return;
        };
        set_saving.set(true);
        set_error_msg.set(None);
        set_saved.set(false);

        let api = api.get();
        let request = UpdateStoreSettingsRequest {
            default_chain_id: None,
            default_display_currency: None,
            logo_url: None,
            accent_color: None,
            notification_prefs: Some(write_matrix(&cells.get())),
        };

        leptos::task::spawn_local(async move {
            let result = api.update_store_settings(&store_id, &request).await;
            // `try_*` throughout: switching tabs disposes this component while
            // the request is still in flight, and writing a disposed signal
            // panics the whole client, which was RCS-220.
            match result {
                Ok(_) => {
                    let _ = set_saved.try_set(true);
                    let _ = set_refresh.try_update(|n| *n += 1);
                }
                Err(e) => {
                    let _ = set_error_msg.try_set(Some(format!("{e}")));
                }
            }
            let _ = set_saving.try_set(false);
        });
    };

    view! {
        <div class="settings-tab-notifications">
            {move || {
                if selected_store_id.get().is_none() {
                    return view! {
                        <NoStoreSelected
                            entity="Notification settings"
                            status=store_status
                            has_stores
                            on_retry=retry_stores
                        />
                    }
                    .into_any();
                }

                view! {
                    <div class="detail-card">
                        <div class="detail-card-header">
                            <h3>"Event notifications"</h3>
                        </div>
                        <div class="detail-card-body">
                            <p class="form-help">
                                "These settings belong to the store selected in the sidebar. \
                                 Switching an event off stops the notification for that channel; \
                                 it does not stop the payment being processed."
                            </p>

                            <Suspense fallback=move || view! {
                                <p class="text-muted">"Loading notification settings..."</p>
                            }>
                                {move || settings.get().map(|result| match result.as_ref() {
                                    Some(Ok(s)) => {
                                        // Populate on the first load for a store
                                        // and on every switch to another one, but
                                        // not on the refresh bump a save triggers,
                                        // which would otherwise re-render the
                                        // matrix from under the merchant.
                                        if loaded_store.get_untracked().as_deref()
                                            != Some(s.store_id.to_string().as_str())
                                        {
                                            cells.set(read_matrix(&s.notification_prefs));
                                            receipts.set(
                                                customer_receipts_enabled(&s.notification_prefs),
                                            );
                                            set_loaded_store.set(Some(s.store_id.to_string()));
                                        }
                                        view! { <NotificationMatrix cells receipts /> }.into_any()
                                    }
                                    Some(Err(e)) => {
                                        let msg = format!("Could not load notification settings: {e}");
                                        view! {
                                            <div class="form-alert form-alert-error">{msg}</div>
                                        }
                                        .into_any()
                                    }
                                    // The `None` arm is unreachable under the
                                    // store guard above, but the resource is
                                    // typed for it.
                                    None => view! { <div></div> }.into_any(),
                                })}
                            </Suspense>

                            {move || error_msg.get().map(|msg| view! {
                                <div class="form-alert form-alert-error">{msg}</div>
                            })}
                            <Show when=move || saved.get()>
                                <div class="form-alert form-alert-success">
                                    "Notification settings saved."
                                </div>
                            </Show>

                            <div class="form-actions">
                                <button
                                    class="btn btn-primary btn-sm"
                                    disabled=move || saving.get() || !ready.get()
                                    on:click=on_save
                                >
                                    {move || if saving.get() { "Saving..." } else { "Save notification settings" }}
                                </button>
                            </div>
                        </div>
                    </div>
                }
                .into_any()
            }}
        </div>
    }
}

/// The event x channel grid.
#[component]
fn NotificationMatrix(
    cells: RwSignal<[bool; NOTIFICATION_EVENTS.len()]>,
    receipts: RwSignal<bool>,
) -> impl IntoView {
    view! {
        <table class="notification-matrix">
            <thead>
                <tr>
                    <th scope="col">"Event"</th>
                    <th scope="col">"Webhook"</th>
                    <th scope="col">"Email"</th>
                </tr>
            </thead>
            <tbody>
                {NOTIFICATION_EVENTS.iter().enumerate().map(|(i, event)| {
                    let email_cell = match event.email {
                        EmailChannel::Unused => view! {
                            <span
                                class="text-muted"
                                title="No email is sent for this event"
                            >
                                "\u{2014}"
                            </span>
                        }
                        .into_any(),
                        // Read-only on purpose: the flag lives at the top level
                        // of the prefs blob, which the PATCH validator rejects,
                        // so a live toggle here could only fail (RCS-228).
                        EmailChannel::CustomerReceipt => view! {
                            <label class="toggle">
                                <input
                                    type="checkbox"
                                    prop:checked=move || receipts.get()
                                    disabled=true
                                    title="Customer receipts cannot be changed here yet"
                                />
                                <span class="toggle-slider"></span>
                            </label>
                        }
                        .into_any(),
                    };

                    view! {
                        <tr>
                            <th scope="row">
                                <span class="notification-option-title">{event.label}</span>
                                <span class="notification-option-desc">{event.description}</span>
                                <code class="notification-event-key">{event.key}</code>
                            </th>
                            <td>
                                <label class="toggle">
                                    <input
                                        type="checkbox"
                                        prop:checked=move || cells.get()[i]
                                        on:change=move |ev| {
                                            let on = event_target_checked(&ev);
                                            cells.update(|c| c[i] = on);
                                        }
                                    />
                                    <span class="toggle-slider"></span>
                                </label>
                            </td>
                            <td>{email_cell}</td>
                        </tr>
                    }
                }).collect_view()}
            </tbody>
        </table>

        <p class="form-help">
            "Webhooks are delivered to the endpoint configured on the store. \
             The only email the server sends is the customer payment receipt, \
             which is why the other email cells are empty rather than off. \
             Receipts are read-only here until the settings endpoint accepts \
             their key (RCS-228)."
        </p>
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CUSTOMER_RECEIPTS_KEY, NOTIFICATION_EVENTS, customer_receipts_enabled, read_matrix,
        webhook_enabled, write_matrix,
    };
    use serde_json::json;

    #[test]
    fn an_unset_blob_reads_as_everything_on() {
        // A store that has never saved this tab gets every webhook, so the
        // matrix must not draw it as everything off.
        let prefs = json!({});
        assert_eq!(read_matrix(&prefs), [true; 5]);
        assert!(customer_receipts_enabled(&prefs));
    }

    #[test]
    fn only_an_explicit_false_reads_as_off() {
        assert!(!webhook_enabled(
            &json!({"late_paid": {"webhook": false}}),
            "late_paid"
        ));
        assert!(webhook_enabled(
            &json!({"late_paid": {"webhook": true}}),
            "late_paid"
        ));
        // Anything the dispatcher would not treat as "off" is on here too.
        assert!(webhook_enabled(&json!({"late_paid": {}}), "late_paid"));
        assert!(webhook_enabled(
            &json!({"late_paid": {"webhook": "false"}}),
            "late_paid"
        ));
        assert!(webhook_enabled(
            &json!({"late_paid": {"email": false}}),
            "late_paid"
        ));
        assert!(webhook_enabled(&json!({}), "late_paid"));
    }

    #[test]
    fn receipts_follow_the_same_rule() {
        assert!(!customer_receipts_enabled(
            &json!({CUSTOMER_RECEIPTS_KEY: false})
        ));
        assert!(customer_receipts_enabled(
            &json!({CUSTOMER_RECEIPTS_KEY: true})
        ));
        assert!(customer_receipts_enabled(
            &json!({CUSTOMER_RECEIPTS_KEY: "no"})
        ));
    }

    #[test]
    fn a_matrix_survives_a_round_trip() {
        let cells = [true, false, false, true, false];
        assert_eq!(read_matrix(&write_matrix(&cells)), cells);
    }

    #[test]
    fn the_payload_carries_every_event_and_only_those() {
        let payload = write_matrix(&[true, false, true, false, true]);
        let obj = payload.as_object().expect("payload is an object");
        assert_eq!(obj.len(), NOTIFICATION_EVENTS.len());
        for event in &NOTIFICATION_EVENTS {
            assert!(obj.contains_key(event.key), "missing {}", event.key);
        }
        // The receipts key must never be sent: the PATCH validator rejects any
        // top-level key it does not know, so including it is a 400.
        assert!(!obj.contains_key(CUSTOMER_RECEIPTS_KEY));
        assert_eq!(payload["payment_detected"], json!({"webhook": true}));
        assert_eq!(payload["payment_confirmed"], json!({"webhook": false}));
    }

    #[test]
    fn rows_are_positional_and_stay_aligned_with_the_payload() {
        // Each cell must land on its own event; an off-by-one here would save
        // the merchant's choices onto the wrong events.
        for i in 0..NOTIFICATION_EVENTS.len() {
            let mut cells = [true; NOTIFICATION_EVENTS.len()];
            cells[i] = false;
            let payload = write_matrix(&cells);
            for (j, event) in NOTIFICATION_EVENTS.iter().enumerate() {
                assert_eq!(
                    payload[event.key]["webhook"],
                    json!(i != j),
                    "event {} at row {i}",
                    event.key
                );
            }
        }
    }
}
