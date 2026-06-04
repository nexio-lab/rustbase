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

use chrono::{NaiveDate, Utc};
use dashmap::DashMap;
use lettre::message::{MultiPart, header::ContentType};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor, message::Mailbox};
use rustbase_core::{AppId, EmailMessage, Mailer, MailerError, PolicySpec, WorkspaceId};
use rustbase_db::{AppPoolManager, policies};
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

/// Policy field name read at every send. Convention; the value is a
/// `PolicySpec::Range` where the upper bound is the effective daily
/// cap and the lower bound is unused.
pub const MAILER_DAILY_QUOTA_FIELD: &str = "mailer.daily_quota";

/// Per-(workspace, app) wrapper that enforces a hierarchical daily-send
/// quota on top of any other `Mailer`. The wrapped mailer (LogMailer
/// or SmtpMailer) handles delivery; this layer adds:
///
/// - A per-day counter keyed by UTC date — rolls over automatically
///   at 00:00 UTC when the next send sees a fresh date key.
/// - A pre-flight policy lookup against the app's `policies` table
///   for `mailer.daily_quota`. The Range's `max` is the cap. Absence
///   of the policy means "no quota", i.e. the wrapper is a pass-through.
/// - Reserve-then-refund semantics: the counter is bumped *before*
///   the transport send; a transport failure refunds the slot so a
///   genuine outage doesn't burn budget.
///
/// System-issued mail (the verify-email + password-reset endpoints)
/// uses the bare `state.mailer` and is *not* quota'd — quotas exist
/// to stop a runaway JS hook from flooding outbound mail, not to
/// throttle the server's own auth flows.
pub struct QuotedMailer {
    inner: Arc<dyn Mailer>,
    workspace: WorkspaceId,
    app: AppId,
    apps: Arc<AppPoolManager>,
    counts: Arc<DashMap<NaiveDate, u32>>,
}

impl QuotedMailer {
    pub fn new(
        inner: Arc<dyn Mailer>,
        workspace: WorkspaceId,
        app: AppId,
        apps: Arc<AppPoolManager>,
    ) -> Self {
        Self {
            inner,
            workspace,
            app,
            apps,
            counts: Arc::new(DashMap::new()),
        }
    }

    /// Resolve the current daily cap, if any. `None` means no policy
    /// row → unlimited. A non-Range policy stored under this field is
    /// also treated as unlimited (with a warning) since the field
    /// convention is Range; we don't want a misconfigured value to
    /// silently block all mail.
    async fn current_quota(&self) -> Result<Option<u32>, MailerError> {
        let pool = self
            .apps
            .pool_for(&self.workspace, &self.app)
            .await
            .map_err(|e| MailerError::Transport(format!("quota pool: {e}")))?;
        let spec = policies::get_policy(&pool, MAILER_DAILY_QUOTA_FIELD)
            .await
            .map_err(|e| MailerError::Transport(format!("quota read: {e}")))?;
        Ok(match spec {
            Some(PolicySpec::Range(r)) if r.max >= 0 => Some(r.max as u32),
            Some(other) => {
                tracing::warn!(
                    workspace = %self.workspace.as_str(),
                    app = %self.app.as_str(),
                    kind = ?other,
                    "mailer.daily_quota policy is not a Range — ignoring"
                );
                None
            }
            None => None,
        })
    }

    /// Snapshot the current day's send count. Test helper; not part of
    /// the `Mailer` contract.
    pub fn count_today(&self) -> u32 {
        let today = Utc::now().date_naive();
        self.counts.get(&today).map(|v| *v).unwrap_or(0)
    }
}

#[async_trait::async_trait]
impl Mailer for QuotedMailer {
    async fn send(&self, msg: EmailMessage) -> Result<(), MailerError> {
        let today = Utc::now().date_naive();
        let quota = self.current_quota().await?;

        // Reserve a slot. If the bump pushes us past the cap, refund
        // immediately and reject. The entry guard is dropped at the end
        // of this scope so we don't hold the DashMap lock across the
        // upcoming await.
        if let Some(q) = quota {
            let mut e = self.counts.entry(today).or_insert(0);
            *e += 1;
            if *e > q {
                *e -= 1;
                return Err(MailerError::Rejected(format!(
                    "daily mail quota of {q} reached for {}/{}",
                    self.workspace.as_str(),
                    self.app.as_str()
                )));
            }
        } else {
            // No quota set — still bump the counter so operators can
            // observe send volume per app via count_today / future
            // metrics, but never reject.
            *self.counts.entry(today).or_insert(0) += 1;
        }

        match self.inner.send(msg).await {
            Ok(()) => Ok(()),
            Err(e) => {
                // Refund the reserved slot on transport failure so
                // operators don't lose mail budget to an SMTP outage.
                if let Some(mut entry) = self.counts.get_mut(&today)
                    && *entry > 0
                {
                    *entry -= 1;
                }
                Err(e)
            }
        }
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

    /// Live SMTP send against the MailHog container from
    /// `infra/docker-compose.yml`. Skipped by default — opt in with:
    ///
    ///     docker compose -f infra/docker-compose.yml up -d
    ///     cargo test -p rustbase-api smtp_mailer_delivers_to_mailhog \
    ///         -- --ignored --nocapture
    ///
    /// A successful `transport.send().await` means MailHog accepted
    /// the DATA segment with a 250 OK — i.e. the message is in the
    /// inbox, browsable at http://localhost:8025. We don't poll the
    /// HTTP API for it; the SMTP transaction itself is the proof.
    #[tokio::test]
    #[ignore = "requires MailHog at localhost:1025 (infra/docker-compose.yml)"]
    async fn smtp_mailer_delivers_to_mailhog() {
        let cfg = SmtpConfig {
            host: "localhost".into(),
            port: 1025,
            username: None,
            password: None,
            tls: SmtpTls::None,
        };
        let mailer = SmtpMailer::new(&cfg).expect("build SmtpMailer");

        let stamp = chrono::Utc::now().timestamp_micros();
        let subject = format!("rustbase mailer smoke {stamp}");

        mailer
            .send(EmailMessage::new(
                "no-reply@rustbase.local",
                "ada@example.com",
                subject,
                "live SMTP send via MailHog",
            ))
            .await
            .expect("MailHog must accept the message");
    }

    /// End-to-end: a JS hook calls `$app.mailer.send(...)` which goes
    /// through SmtpMailer → lettre → MailHog. Same opt-in as the
    /// bare SMTP smoke test: `--ignored`, with MailHog running.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "requires MailHog at localhost:1025 (infra/docker-compose.yml)"]
    async fn hook_mailer_send_delivers_to_mailhog() {
        use rustbase_core::Mailer;
        use rustbase_runtime::{AppHooks, SandboxLimits};
        use std::sync::Arc;

        let cfg = SmtpConfig {
            host: "localhost".into(),
            port: 1025,
            username: None,
            password: None,
            tls: SmtpTls::None,
        };
        let smtp: Arc<dyn Mailer> = Arc::new(SmtpMailer::new(&cfg).unwrap());
        let hooks =
            AppHooks::with_records_mailer_and_limits(None, Some(smtp), SandboxLimits::default())
                .await
                .unwrap();

        let stamp = chrono::Utc::now().timestamp_micros();
        let src = format!(
            r#"
            $app.mailer.send({{
                from: "no-reply@rustbase.local",
                to:   "hook-test@example.com",
                subject: "from $app.mailer.send #{stamp}",
                text: "this came from a hook"
            }});
            $app.log("dispatched");
            "#
        );
        hooks
            .eval(&src, "<hook-smoke>")
            .await
            .expect("hook eval must succeed");
        let logs = hooks.drain_logs().await.unwrap();
        assert_eq!(logs, vec!["dispatched".to_string()]);
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

    // ------------- QuotedMailer -------------

    use rustbase_core::{PolicySpec, RangePolicy};
    use rustbase_db::migrations::{APP_MIGRATIONS, WORKSPACE_MIGRATIONS, apply_migrations};
    use rustbase_db::pool::AppPoolManager;
    use std::path::PathBuf;
    use tempfile::tempdir;

    /// Spin up a temp on-disk universe and pre-seed the workspace + app DBs.
    /// Returns the app pool manager plus the (workspace, app) keys so a
    /// QuotedMailer can be parameterised.
    async fn fresh_app_universe() -> (
        Arc<AppPoolManager>,
        WorkspaceId,
        AppId,
        tempfile::TempDir,
        sqlx::SqlitePool,
    ) {
        let dir = tempdir().unwrap();
        let data_dir: PathBuf = dir.path().to_path_buf();
        // Workspace DB needs its migrations so the apps table exists for
        // pool resolution; app DB needs its migrations so the
        // _policies table exists for QuotedMailer's policy lookup.
        let workspace_pools = Arc::new(rustbase_db::pool::WorkspacePoolManager::new(
            data_dir.clone(),
            4,
        ));
        let workspace = WorkspaceId::from("acme");
        let app = AppId::from("mobile");
        let workspace_pool = workspace_pools.pool_for(&workspace).await.unwrap();
        apply_migrations(workspace_pool.clone(), WORKSPACE_MIGRATIONS)
            .await
            .unwrap();

        let apps = Arc::new(AppPoolManager::new(data_dir.clone(), 4));
        let app_pool = apps.pool_for(&workspace, &app).await.unwrap();
        apply_migrations(app_pool.clone(), APP_MIGRATIONS)
            .await
            .unwrap();
        (apps, workspace, app, dir, app_pool)
    }

    /// Counting MailerError-clean transport. Bare Vec under a mutex
    /// keeps the asserts simple.
    #[derive(Default)]
    struct CountingMailer {
        sent: parking_lot::Mutex<Vec<EmailMessage>>,
    }
    #[async_trait::async_trait]
    impl Mailer for CountingMailer {
        async fn send(&self, msg: EmailMessage) -> Result<(), MailerError> {
            self.sent.lock().push(msg);
            Ok(())
        }
    }

    /// Always-fails transport for refund-on-error coverage.
    struct FailingMailer;
    #[async_trait::async_trait]
    impl Mailer for FailingMailer {
        async fn send(&self, _msg: EmailMessage) -> Result<(), MailerError> {
            Err(MailerError::Transport("simulated outage".into()))
        }
    }

    fn quota_spec(max: i64) -> PolicySpec {
        PolicySpec::Range(RangePolicy::new(0, max).unwrap())
    }

    fn msg() -> EmailMessage {
        EmailMessage::new("a@x", "b@y", "s", "body")
    }

    #[tokio::test]
    async fn quoted_mailer_passes_through_when_no_policy_set() {
        let (apps, workspace, app, _dir, _app_pool) = fresh_app_universe().await;
        let inner = Arc::new(CountingMailer::default());
        let qm = QuotedMailer::new(inner.clone(), workspace, app, apps);

        // 100 sends, no policy → all succeed (unlimited).
        for _ in 0..100 {
            qm.send(msg()).await.unwrap();
        }
        assert_eq!(inner.sent.lock().len(), 100);
        assert_eq!(qm.count_today(), 100);
    }

    #[tokio::test]
    async fn quoted_mailer_enforces_daily_cap() {
        let (apps, workspace, app, _dir, app_pool) = fresh_app_universe().await;
        policies::upsert_policy(&app_pool, MAILER_DAILY_QUOTA_FIELD, &quota_spec(3))
            .await
            .unwrap();

        let inner = Arc::new(CountingMailer::default());
        let qm = QuotedMailer::new(inner.clone(), workspace, app, apps);
        // Three sends succeed, fourth must be Rejected with the cap
        // in the message.
        for _ in 0..3 {
            qm.send(msg()).await.unwrap();
        }
        let err = qm.send(msg()).await.unwrap_err();
        match err {
            MailerError::Rejected(m) => assert!(m.contains("quota of 3"), "got: {m}"),
            other => panic!("expected Rejected, got {other:?}"),
        }
        assert_eq!(
            inner.sent.lock().len(),
            3,
            "fourth must not reach transport"
        );
        assert_eq!(qm.count_today(), 3);
    }

    #[tokio::test]
    async fn quoted_mailer_refunds_slot_on_transport_failure() {
        let (apps, workspace, app, _dir, app_pool) = fresh_app_universe().await;
        policies::upsert_policy(&app_pool, MAILER_DAILY_QUOTA_FIELD, &quota_spec(2))
            .await
            .unwrap();

        let qm = QuotedMailer::new(Arc::new(FailingMailer), workspace, app, apps);
        // Three failing sends → each refunds; count stays at 0.
        for _ in 0..3 {
            let err = qm.send(msg()).await.unwrap_err();
            assert!(matches!(err, MailerError::Transport(_)));
        }
        assert_eq!(qm.count_today(), 0, "failed sends must not burn budget");
    }

    #[tokio::test]
    async fn quoted_mailer_ignores_non_range_policy() {
        // A misconfigured field (Toggle instead of Range) should not
        // silently lock out all mail; we log + treat as unlimited.
        let (apps, workspace, app, _dir, app_pool) = fresh_app_universe().await;
        policies::upsert_policy(
            &app_pool,
            MAILER_DAILY_QUOTA_FIELD,
            &PolicySpec::Toggle(rustbase_core::TogglePolicy::Locked { value: true }),
        )
        .await
        .unwrap();

        let inner = Arc::new(CountingMailer::default());
        let qm = QuotedMailer::new(inner.clone(), workspace, app, apps);
        for _ in 0..5 {
            qm.send(msg()).await.unwrap();
        }
        assert_eq!(inner.sent.lock().len(), 5);
    }
}
