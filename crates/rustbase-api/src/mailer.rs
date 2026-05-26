//! `Mailer` implementations for the API layer.
//!
//! Two impls ship here:
//!
//! - [`LogMailer`] — in-memory capture for dev and tests. Logs every
//!   send through `tracing::info!` and stashes the message in an
//!   `Arc<Mutex<Vec<_>>>` so tests can assert on delivery without
//!   touching the network. Server boots with this when no SMTP config
//!   is present.
//! - [`SmtpMailer`] — production transport backed by `lettre`. Holds
//!   one async SMTP connection pool keyed by host/port and optional
//!   credentials. Selected at boot when `[mail.smtp]` is configured.

use lettre::message::{MultiPart, header::ContentType};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor, message::Mailbox};
use rustbase_core::{EmailMessage, Mailer, MailerError};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::Mutex;

/// In-memory mailer. Captures every send for assertions and logs each
/// one through `tracing::info!` so dev runs surface what would have
/// been delivered.
#[derive(Default, Clone)]
pub struct LogMailer {
    sent: Arc<Mutex<Vec<EmailMessage>>>,
}

impl LogMailer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot of every message that has been sent through this
    /// mailer since construction. Returns clones so the caller can
    /// inspect without holding the lock.
    pub async fn sent(&self) -> Vec<EmailMessage> {
        self.sent.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl Mailer for LogMailer {
    async fn send(&self, msg: EmailMessage) -> Result<(), MailerError> {
        tracing::info!(
            from = %msg.from,
            to = %msg.to,
            subject = %msg.subject,
            "[LogMailer] outbound message"
        );
        self.sent.lock().await.push(msg);
        Ok(())
    }
}

/// Wire-level TLS posture for the SMTP connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SmtpTls {
    /// Plain TCP. Only safe on localhost or inside a trusted network.
    None,
    /// Connect plaintext, upgrade with STARTTLS (port 587 by default).
    #[default]
    StartTls,
    /// Implicit TLS from byte zero (port 465 by default).
    Implicit,
}

/// Configuration for an SMTP relay. Pulled out of `rustbase.toml` by
/// the server crate; `[mail.smtp]` absence boots the server with a
/// `LogMailer` instead.
#[derive(Debug, Clone, Deserialize)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    /// SASL username. Optional for unauthenticated relays (e.g. a
    /// localhost MTA that gates by source IP).
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub tls: SmtpTls,
}

/// Lettre-backed `Mailer`. Owns the async transport pool for the
/// lifetime of the process; messages are dispatched on the caller's
/// tokio runtime.
pub struct SmtpMailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
}

impl SmtpMailer {
    pub fn new(cfg: &SmtpConfig) -> Result<Self, MailerError> {
        let mut builder = match cfg.tls {
            SmtpTls::Implicit => AsyncSmtpTransport::<Tokio1Executor>::relay(&cfg.host)
                .map_err(|e| MailerError::Transport(e.to_string()))?,
            SmtpTls::StartTls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.host)
                .map_err(|e| MailerError::Transport(e.to_string()))?,
            SmtpTls::None => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&cfg.host),
        };
        builder = builder.port(cfg.port);
        if let (Some(u), Some(p)) = (cfg.username.as_ref(), cfg.password.as_ref()) {
            builder = builder.credentials(Credentials::new(u.clone(), p.clone()));
        }
        Ok(Self {
            transport: builder.build(),
        })
    }
}

#[async_trait::async_trait]
impl Mailer for SmtpMailer {
    async fn send(&self, msg: EmailMessage) -> Result<(), MailerError> {
        let from: Mailbox = msg
            .from
            .parse()
            .map_err(|e| MailerError::Rejected(format!("invalid from address: {e}")))?;
        let to: Mailbox = msg
            .to
            .parse()
            .map_err(|e| MailerError::Rejected(format!("invalid to address: {e}")))?;

        let builder = Message::builder().from(from).to(to).subject(&msg.subject);

        let email = match &msg.html {
            Some(html) => builder
                .multipart(MultiPart::alternative_plain_html(
                    msg.text.clone(),
                    html.clone(),
                ))
                .map_err(|e| MailerError::Rejected(format!("multipart: {e}")))?,
            None => builder
                .header(ContentType::TEXT_PLAIN)
                .body(msg.text.clone())
                .map_err(|e| MailerError::Rejected(format!("plain body: {e}")))?,
        };

        self.transport
            .send(email)
            .await
            .map_err(|e| MailerError::Transport(e.to_string()))?;
        tracing::info!(
            to = %msg.to,
            subject = %msg.subject,
            "[SmtpMailer] sent"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn captures_each_send_in_order() {
        let mailer = LogMailer::new();
        mailer
            .send(EmailMessage::new("a@x", "b@y", "first", "hello"))
            .await
            .unwrap();
        mailer
            .send(EmailMessage::new("a@x", "c@y", "second", "world"))
            .await
            .unwrap();
        let sent = mailer.sent().await;
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].subject, "first");
        assert_eq!(sent[1].to, "c@y");
    }

    #[tokio::test]
    async fn clones_share_the_capture_buffer() {
        let mailer = LogMailer::new();
        let clone = mailer.clone();
        clone
            .send(EmailMessage::new("a@x", "b@y", "subj", "body"))
            .await
            .unwrap();
        assert_eq!(mailer.sent().await.len(), 1);
    }

    fn smtp_cfg(tls: SmtpTls) -> SmtpConfig {
        SmtpConfig {
            host: "smtp.example.test".into(),
            port: 587,
            username: Some("user".into()),
            password: Some("pw".into()),
            tls,
        }
    }

    #[test]
    fn smtp_mailer_constructs_for_each_tls_mode() {
        // Building the transport doesn't touch the network — it just
        // sets up the connection pool config. We're verifying that the
        // lettre helpers we picked accept our shape.
        for tls in [SmtpTls::None, SmtpTls::StartTls, SmtpTls::Implicit] {
            let mailer = SmtpMailer::new(&smtp_cfg(tls));
            assert!(mailer.is_ok(), "failed for {tls:?}: {:?}", mailer.err());
        }
    }

    #[test]
    fn smtp_mailer_accepts_anonymous_relay() {
        let mut cfg = smtp_cfg(SmtpTls::None);
        cfg.username = None;
        cfg.password = None;
        SmtpMailer::new(&cfg).expect("anonymous relay should build");
    }

    #[tokio::test]
    async fn smtp_mailer_rejects_malformed_addresses_without_network() {
        // Pointed at a sink that will never accept; reaching transport
        // means we got past local validation, which is what this test
        // intentionally avoids. Bad From: fails before any I/O.
        let mailer = SmtpMailer::new(&smtp_cfg(SmtpTls::None)).unwrap();
        let bad = EmailMessage::new("not an email", "to@example.test", "subj", "body");
        let err = mailer.send(bad).await.unwrap_err();
        assert!(
            matches!(err, MailerError::Rejected(_)),
            "expected Rejected, got: {err:?}"
        );
    }
}
