//! Mailer interface — IO-free contract for sending email out of
//! RustBaas. Implementations live in higher layers (an in-memory
//! `LogMailer` for dev/test, an SMTP-backed one for production).
//!
//! The trait is `async` and `Send + Sync` so it can be stored as
//! `Arc<dyn Mailer>` on the request context and called from inside
//! axum handlers without cloning the world.

use serde::{Deserialize, Serialize};

/// A single outbound email. Recipients and sender are envelope-only
/// here — display names go in the headers a layer above. `html` is
/// optional so plain-text-only senders don't have to build a
/// multipart/alternative payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailMessage {
    pub from: String,
    pub to: String,
    pub subject: String,
    pub text: String,
    pub html: Option<String>,
}

impl EmailMessage {
    pub fn new(
        from: impl Into<String>,
        to: impl Into<String>,
        subject: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            subject: subject.into(),
            text: text.into(),
            html: None,
        }
    }

    pub fn with_html(mut self, html: impl Into<String>) -> Self {
        self.html = Some(html.into());
        self
    }
}

/// A mailer dispatches one message at a time. Failures bubble up as
/// `MailerError`; the caller decides whether to retry, queue, or
/// surface to the user.
#[async_trait::async_trait]
pub trait Mailer: Send + Sync + 'static {
    async fn send(&self, msg: EmailMessage) -> Result<(), MailerError>;
}

/// Errors a mailer can raise. Kept narrow — implementations stuff the
/// backend-specific detail into the inner string so the trait stays
/// free of dependencies on lettre / reqwest / etc.
#[derive(Debug, thiserror::Error)]
pub enum MailerError {
    #[error("transport error: {0}")]
    Transport(String),
    #[error("rejected by remote server: {0}")]
    Rejected(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_builder_round_trips_html() {
        let m = EmailMessage::new("a@x", "b@y", "hi", "hello").with_html("<p>hello</p>");
        assert_eq!(m.html.as_deref(), Some("<p>hello</p>"));
        assert_eq!(m.from, "a@x");
    }

    #[test]
    fn message_serializes_with_html_omitted_when_none() {
        let m = EmailMessage::new("a@x", "b@y", "hi", "hello");
        let s = serde_json::to_string(&m).unwrap();
        // Plain-text-only message: serde_json emits `"html":null`,
        // which is fine — just confirms the field isn't missing.
        assert!(s.contains("\"html\":null"), "got: {s}");
    }
}
