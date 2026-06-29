//! Payment-related tool implementations.

use chrono::Utc;

use types::{InvoiceId, InvoiceStatus, InvoiceWriter, PaymentOptionWriter, PaymentReader};

use super::EthpayMcpServer;
use super::args::{CancelInvoiceArgs, GetInvoicePaymentsArgs, GetPaymentStatusArgs};

impl EthpayMcpServer {
    pub(super) async fn do_get_invoice_payments(
        &self,
        args: GetInvoicePaymentsArgs,
    ) -> Result<String, String> {
        let id = InvoiceId::from_string(args.invoice_id);
        let _invoice = self.authorize_invoice(&id).await?;

        let payments = PaymentReader::get_for_invoice(&*self.data_service, &id)
            .await
            .map_err(|e| format!("Failed to get payments: {e}"))?;

        let result = serde_json::json!({
            "invoice_id": id.0,
            "payments": payments.iter().map(|p| serde_json::json!({
                "id": p.id.to_string(),
                "chain_id": p.chain_id,
                "asset_symbol": p.asset_symbol,
                "amount": p.amount,
                "tx_hash": p.tx_hash,
                "block_number": p.block_number,
                "from_address": p.from_address,
                "detected_at": p.detected_at.to_rfc3339(),
                "confirmed_at": p.confirmed_at.map(|t| t.to_rfc3339()),
                "reorged": p.reorged,
                "credited_amount": p.credited_amount,
                "rate_used": p.rate_used,
            })).collect::<Vec<_>>(),
        });

        Ok(serde_json::to_string_pretty(&result).unwrap())
    }

    pub(super) async fn do_cancel_invoice(&self, args: CancelInvoiceArgs) -> Result<String, String> {
        let id = InvoiceId::from_string(args.invoice_id);
        let invoice = self.authorize_invoice(&id).await?;

        match invoice.status {
            InvoiceStatus::Pending | InvoiceStatus::Processing | InvoiceStatus::PartiallyPaid => {}
            other => return Err(format!("Cannot cancel invoice in '{}' status", other)),
        }

        InvoiceWriter::update_status(&*self.data_service, &id, InvoiceStatus::Cancelled)
            .await
            .map_err(|e| format!("Failed to cancel: {e}"))?;

        // Deactivate payment options
        let _ = PaymentOptionWriter::deactivate_for_invoice(&*self.data_service, &id).await;

        Ok(serde_json::json!({
            "id": id.0,
            "status": "cancelled",
            "previous_status": invoice.status.to_string(),
        })
        .to_string())
    }

    pub(super) async fn do_get_payment_status(
        &self,
        args: GetPaymentStatusArgs,
    ) -> Result<String, String> {
        let id = InvoiceId::from_string(args.invoice_id);
        let invoice = self.authorize_invoice(&id).await?;

        let payments = PaymentReader::get_for_invoice(&*self.data_service, &id)
            .await
            .map_err(|e| format!("Failed to get payments: {e}"))?;

        let now = Utc::now();
        let confirmed_count = payments.iter().filter(|p| p.confirmed_at.is_some()).count();

        // Compute simplified status
        let simplified_status = match invoice.status {
            InvoiceStatus::Paid => "settled",
            InvoiceStatus::Expired => "expired",
            InvoiceStatus::Cancelled => "cancelled",
            InvoiceStatus::LatePaid => "settled",
            InvoiceStatus::Refunded => "refunded",
            InvoiceStatus::PartiallyPaid => "partial",
            _ if invoice.expires_at < now => "expired",
            _ => "pending",
        };

        let result = serde_json::json!({
            "invoice_id": id.0,
            "status": simplified_status,
            "amount": invoice.amount,
            "amount_received": invoice.amount_received,
            "currency": invoice.currency,
            "expires_at": invoice.expires_at.to_rfc3339(),
            "payment_count": payments.len(),
            "confirmed_count": confirmed_count,
            "is_paid": invoice.status == InvoiceStatus::Paid
                || invoice.status == InvoiceStatus::LatePaid,
            "is_expired": invoice.status == InvoiceStatus::Expired || invoice.expires_at < now,
        });

        Ok(serde_json::to_string_pretty(&result).unwrap())
    }
}
