//! `Mailer` implementations for the API layer.
//!
//! Production SMTP support is deliberately deferred — adding a real
//! mail transport pulls in lettre / TLS / DNS resolution, and the
//! contract this branch needs to validate (verify-email flow) is fully
//! exercised by a capture-only mailer. `LogMailer` records every
//! outbound message in an `Arc<Mutex<Vec<…>>>` so tests can assert on
//! delivery without touching the network.

use rustbase_core::{EmailMessage, Mailer, MailerError};
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
}
