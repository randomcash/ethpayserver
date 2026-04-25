//! Email service for sending payment receipts to customers.
//!
//! Configured via environment variables. When SMTP is not configured,
//! the service is a no-op.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

/// SMTP configuration parsed from environment variables.
#[derive(Debug, Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub from: String,
}

impl SmtpConfig {
    /// Try to load SMTP config from environment. Returns None if required
    /// variables are not set (SMTP is optional).
    pub fn from_env() -> Option<Self> {
        let host = std::env::var("SMTP_HOST").ok()?;
        let port = std::env::var("SMTP_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(587);
        let username = std::env::var("SMTP_USERNAME").ok()?;
        let password = std::env::var("SMTP_PASSWORD").ok()?;
        let from = std::env::var("SMTP_FROM").unwrap_or_else(|_| format!("noreply@{host}"));
        Some(Self {
            host,
            port,
            username,
            password,
            from,
        })
    }
}

/// Receipt data needed to compose the email.
pub struct ReceiptData {
    pub invoice_id: String,
    pub amount: String,
    pub currency: String,
    pub tx_hash: String,
    pub explorer_url: String,
    pub paid_at: DateTime<Utc>,
    pub merchant_name: String,
}

/// Email service that sends payment receipts via SMTP.
pub struct EmailService {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: String,
}

impl EmailService {
    /// Build an `EmailService` from an `SmtpConfig`.
    pub fn new(config: &SmtpConfig) -> Result<Self, lettre::transport::smtp::Error> {
        let creds = Credentials::new(config.username.clone(), config.password.clone());
        let transport = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.host)?
            .port(config.port)
            .credentials(creds)
            .build();
        Ok(Self {
            transport,
            from: config.from.clone(),
        })
    }

    /// Send a payment receipt to the customer.
    pub async fn send_receipt(&self, to: &str, data: &ReceiptData) -> Result<(), EmailError> {
        let subject = format!(
            "Payment receipt — {} {} ({})",
            data.amount, data.currency, data.invoice_id
        );

        let body = format!(
            "Payment Receipt\n\
             ================\n\
             \n\
             Merchant:    {merchant}\n\
             Invoice:     {invoice}\n\
             Amount:      {amount} {currency}\n\
             Transaction: {tx}\n\
             Explorer:    {explorer}\n\
             Paid at:     {paid_at}\n\
             \n\
             This is an automated receipt from {merchant} powered by random.cash.\n\
             If you did not make this payment, please contact the merchant.\n",
            merchant = data.merchant_name,
            invoice = data.invoice_id,
            amount = data.amount,
            currency = data.currency,
            tx = data.tx_hash,
            explorer = data.explorer_url,
            paid_at = data.paid_at.format("%Y-%m-%d %H:%M:%S UTC"),
        );

        let email = Message::builder()
            .from(self.from.parse().map_err(|_| EmailError::InvalidFrom)?)
            .to(to.parse().map_err(|_| EmailError::InvalidRecipient)?)
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .body(body)
            .map_err(|e| EmailError::Build(e.to_string()))?;

        self.transport
            .send(email)
            .await
            .map_err(|e| EmailError::Send(e.to_string()))?;

        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EmailError {
    #[error("invalid from address")]
    InvalidFrom,
    #[error("invalid recipient address")]
    InvalidRecipient,
    #[error("email build error: {0}")]
    Build(String),
    #[error("smtp send error: {0}")]
    Send(String),
}

/// Trait abstracting email sending for testability.
#[async_trait::async_trait]
pub trait EmailSender: Send + Sync {
    async fn send_receipt(&self, to: &str, data: &ReceiptData) -> Result<(), EmailError>;
}

#[async_trait::async_trait]
impl EmailSender for EmailService {
    async fn send_receipt(&self, to: &str, data: &ReceiptData) -> Result<(), EmailError> {
        self.send_receipt(to, data).await
    }
}

/// No-op email sender for when SMTP is not configured.
pub struct NoopEmailSender;

#[async_trait::async_trait]
impl EmailSender for NoopEmailSender {
    async fn send_receipt(&self, _to: &str, _data: &ReceiptData) -> Result<(), EmailError> {
        Ok(())
    }
}

/// Create the appropriate email sender based on environment configuration.
pub fn create_email_sender() -> Arc<dyn EmailSender> {
    match SmtpConfig::from_env() {
        Some(config) => match EmailService::new(&config) {
            Ok(service) => {
                tracing::info!(host = %config.host, "SMTP email service configured");
                Arc::new(service)
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to configure SMTP, receipts disabled");
                Arc::new(NoopEmailSender)
            }
        },
        None => {
            tracing::info!("SMTP not configured, customer receipts disabled");
            Arc::new(NoopEmailSender)
        }
    }
}
