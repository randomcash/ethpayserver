//! Wire messages and connection state for the status WebSocket.

use serde::{Deserialize, Serialize};

/// Real-time status update message from server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StatusUpdate {
    /// Invoice status changed.
    #[serde(rename = "invoice_status")]
    InvoiceStatus { invoice_id: String, status: String },
    /// Payment received or updated.
    #[serde(rename = "payment_update")]
    PaymentUpdate {
        payment_id: String,
        invoice_id: String,
        status: String,
        amount: Option<String>,
    },
    /// Connection acknowledged.
    #[serde(rename = "connected")]
    Connected,
    /// Server-sent ping.
    #[serde(rename = "ping")]
    Ping,
}

/// WebSocket connection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// Not connected and not attempting to connect.
    Disconnected,
    /// Actively connected to the server.
    Connected,
    /// Connection lost, attempting to reconnect.
    Reconnecting,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_update_serde_invoice_status() {
        let update = StatusUpdate::InvoiceStatus {
            invoice_id: "inv_1".to_string(),
            status: "paid".to_string(),
        };
        let json = serde_json::to_string(&update).unwrap();
        let parsed: StatusUpdate = serde_json::from_str(&json).unwrap();
        assert!(
            matches!(parsed, StatusUpdate::InvoiceStatus { invoice_id, status } if invoice_id == "inv_1" && status == "paid")
        );
    }

    #[test]
    fn test_status_update_serde_payment_update_with_amount() {
        let update = StatusUpdate::PaymentUpdate {
            payment_id: "pay_1".to_string(),
            invoice_id: "inv_1".to_string(),
            status: "confirmed".to_string(),
            amount: Some("1.5".to_string()),
        };
        let json = serde_json::to_value(&update).unwrap();
        assert_eq!(json["type"], "payment_update");
        assert_eq!(json["amount"], "1.5");
    }

    #[test]
    fn test_status_update_serde_payment_update_without_amount() {
        let update = StatusUpdate::PaymentUpdate {
            payment_id: "pay_2".to_string(),
            invoice_id: "inv_2".to_string(),
            status: "detecting".to_string(),
            amount: None,
        };
        let json = serde_json::to_value(&update).unwrap();
        assert!(json.get("amount").unwrap().is_null());
    }

    #[test]
    fn test_status_update_connected_and_ping() {
        assert_eq!(
            serde_json::to_string(&StatusUpdate::Connected).unwrap(),
            r#"{"type":"connected"}"#
        );
        assert_eq!(
            serde_json::to_string(&StatusUpdate::Ping).unwrap(),
            r#"{"type":"ping"}"#
        );
    }

    #[test]
    fn test_status_update_roundtrip_all_variants() {
        let variants = vec![
            StatusUpdate::InvoiceStatus {
                invoice_id: "i".to_string(),
                status: "s".to_string(),
            },
            StatusUpdate::PaymentUpdate {
                payment_id: "p".to_string(),
                invoice_id: "i".to_string(),
                status: "s".to_string(),
                amount: None,
            },
            StatusUpdate::Connected,
            StatusUpdate::Ping,
        ];
        for v in variants {
            let json = serde_json::to_string(&v).unwrap();
            let _: StatusUpdate = serde_json::from_str(&json).unwrap();
        }
    }

    /// Mirror of the server-side `test_status_update_json_contract` test.
    /// Both tests assert the exact same JSON so any serde-attribute drift
    /// between client and server `StatusUpdate` definitions is caught.
    #[test]
    fn test_status_update_json_contract() {
        // Connected
        assert_eq!(
            serde_json::to_string(&StatusUpdate::Connected).unwrap(),
            r#"{"type":"connected"}"#,
        );

        // Ping
        assert_eq!(
            serde_json::to_string(&StatusUpdate::Ping).unwrap(),
            r#"{"type":"ping"}"#,
        );

        // InvoiceStatus
        let invoice = StatusUpdate::InvoiceStatus {
            invoice_id: "inv_1".to_string(),
            status: "paid".to_string(),
        };
        let json = serde_json::to_value(&invoice).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "type": "invoice_status",
                "invoice_id": "inv_1",
                "status": "paid"
            })
        );

        // PaymentUpdate with amount
        let payment = StatusUpdate::PaymentUpdate {
            payment_id: "pay_1".to_string(),
            invoice_id: "inv_1".to_string(),
            status: "confirmed".to_string(),
            amount: Some("1.5".to_string()),
        };
        let json = serde_json::to_value(&payment).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "type": "payment_update",
                "payment_id": "pay_1",
                "invoice_id": "inv_1",
                "status": "confirmed",
                "amount": "1.5"
            })
        );

        // PaymentUpdate without amount
        let payment_no_amt = StatusUpdate::PaymentUpdate {
            payment_id: "pay_2".to_string(),
            invoice_id: "inv_2".to_string(),
            status: "detecting".to_string(),
            amount: None,
        };
        let json = serde_json::to_value(&payment_no_amt).unwrap();
        // Use .get().unwrap().is_null() instead of json["amount"] == Null
        // so this assertion catches a future skip_serializing_if annotation
        // that would omit the key entirely.
        assert!(json.get("amount").unwrap().is_null());
    }

    #[test]
    fn test_connection_state_default() {
        assert_eq!(ConnectionState::Disconnected, ConnectionState::Disconnected);
        assert_ne!(ConnectionState::Connected, ConnectionState::Disconnected);
        assert_ne!(ConnectionState::Reconnecting, ConnectionState::Connected);
    }
}
