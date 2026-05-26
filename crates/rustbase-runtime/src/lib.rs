//! Embedded JS hook runtime for RustBase.
//!
//! One `AppHooks` per (realm, app). Each holds its own `AsyncRuntime`
//! and `AsyncContext`. At load time we evaluate every `.js` file under
//! `data/hooks/<realm>/<app>/` against the context; user code registers
//! handlers via the injected `$app` global:
//!
//! ```js
//! $app.onRecordAfterCreate("notes", (record) => {
//!   $app.log("created note " + record.id);
//! });
//! ```
//!
//! When a record CRUD endpoint succeeds, the API layer calls
//! `HookEngine::dispatch(...)`. We look up the AppHooks for the
//! (realm, app), then evaluate a tiny driver script that pulls
//! handlers out of a global handler table and invokes them with the
//! record JSON. Errors inside a hook are caught + stashed; they don't
//! fail the HTTP request — hooks are post-write observers, not
//! transactions.
//!
//! Out of scope on this branch: TypeScript transpile, sandbox limits
//! (CPU / memory / fs / network), `$app.records.*` data API. Each
//! follow-up branch can extend the `$app` surface in-place.

use async_trait::async_trait;
use dashmap::DashMap;
use rquickjs::{AsyncContext, AsyncRuntime, CatchResultExt};
use rustbase_core::{EmailMessage, Mailer};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use thiserror::Error;

mod sandbox;
use sandbox::CpuClock;
pub use sandbox::SandboxLimits;

mod ts;

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("js error: {0}")]
    Js(String),
    /// A before-hook threw, vetoing the request.
    #[error("vetoed by hook: {0}")]
    Veto(String),
    /// CPU deadline elapsed mid-execution. Raised when a JS entry
    /// returns an error and the per-call deadline had been crossed —
    /// the QuickJS interrupt handler aborted the running script.
    #[error("hook exceeded cpu deadline ({0} ms)")]
    Timeout(u64),
}

pub type Result<T> = std::result::Result<T, RuntimeError>;

impl From<rquickjs::Error> for RuntimeError {
    fn from(e: rquickjs::Error) -> Self {
        RuntimeError::Js(e.to_string())
    }
}

/// Per-dispatch request context exposed to hooks as `$app.request`.
///
/// Built fresh on every CRUD handler entry and threaded through to
/// the JS runtime. While dispatch is running, `$app.request` reflects
/// THIS request; once dispatch returns, it's nulled so a later
/// internal call (e.g. from `$app.records.create` inside the same
/// context) doesn't see a stale principal.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HookRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<HookAuth>,
    pub realm: String,
    pub app: String,
    pub collection: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HookAuth {
    pub id: String,
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub realm: Option<String>,
}

impl HookRequest {
    /// A blank request — used for tests and for internal callers
    /// (e.g. the bridge) that don't have an authenticated principal.
    pub fn system(realm: &str, app: &str, collection: &str) -> Self {
        Self {
            auth: None,
            realm: realm.to_string(),
            app: app.to_string(),
            collection: collection.to_string(),
        }
    }
}

/// Request context handed to a `$app.routerAdd` handler at invocation
/// time. Serialised to JSON, deserialised inside the JS runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomRouteContext {
    pub method: String,
    pub path: String,
    /// Flat query map. Repeated keys keep only the last value — the
    /// usual axum convention. Empty when the path had no query string.
    pub query: BTreeMap<String, String>,
    /// Lowercased header names; values are the raw `&str` form.
    pub headers: BTreeMap<String, String>,
    /// Parsed JSON body when `Content-Type` was `application/json`
    /// AND the body parsed cleanly. `null` otherwise (no body, wrong
    /// content type, malformed JSON).
    pub body: Json,
}

/// JSON response shape produced by a `routerAdd` handler. Defaults
/// match the JS shim — `status` defaults to 200 inside the JS adapter
/// before this struct is parsed on the Rust side, so the Option is
/// here purely for malformed inputs we want to tolerate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomRouteResponse {
    #[serde(default = "default_status")]
    pub status: u16,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body: Json,
}

fn default_status() -> u16 {
    200
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    AfterCreate,
    AfterUpdate,
    AfterDelete,
    BeforeCreate,
    BeforeUpdate,
    BeforeDelete,
    /// Fires after an authentication track has validated credentials
    /// but before access/refresh tokens are issued. Vetoable: throwing
    /// from this handler aborts the login. The "collection" passed at
    /// dispatch is always the sentinel [`USER_HOOK_COLLECTION`].
    UserBeforeLogin,
    /// Fires after a user has been successfully logged in (any track:
    /// password, OTP, OAuth). Observer only.
    UserAfterLogin,
    /// Fires after a fresh user row has been inserted (any signup
    /// track: /register, OTP first-time, OAuth first-time). Observer
    /// only.
    UserAfterRegister,
}

/// Pseudo-collection name used as the routing key for realm-wide
/// user-lifecycle hooks. Records can't use this name (the API
/// already rejects identifiers starting with `_`).
pub const USER_HOOK_COLLECTION: &str = "_user";

impl HookEvent {
    fn as_str(&self) -> &'static str {
        match self {
            HookEvent::AfterCreate => "after_create",
            HookEvent::AfterUpdate => "after_update",
            HookEvent::AfterDelete => "after_delete",
            HookEvent::BeforeCreate => "before_create",
            HookEvent::BeforeUpdate => "before_update",
            HookEvent::BeforeDelete => "before_delete",
            HookEvent::UserBeforeLogin => "user_before_login",
            HookEvent::UserAfterLogin => "user_after_login",
            HookEvent::UserAfterRegister => "user_after_register",
        }
    }
}

/// The thin slice of the records layer the JS runtime needs.
///
/// Implemented by `rustbase-api` (or any test double); kept here as a
/// trait so this crate doesn't depend on the API/db crates and can
/// stand alone in tests. All methods are SYNC — the JS runtime is
/// dispatched on a tokio worker thread, and these implementations
/// will internally `block_in_place + Handle::block_on(...)` to call
/// the async DB layer.
pub trait RecordsBridge: Send + Sync + 'static {
    fn find_one(&self, collection: &str, id: &str) -> Result<Option<Json>>;
    fn find_by_filter(&self, collection: &str, filter: &str, per_page: u32) -> Result<Vec<Json>>;
    fn create(&self, collection: &str, fields: BTreeMap<String, Json>) -> Result<Json>;
    fn update(&self, collection: &str, id: &str, patch: BTreeMap<String, Json>) -> Result<Json>;
    fn delete(&self, collection: &str, id: &str) -> Result<()>;
}

/// `async_trait` form for tests that want to write the bridge once
/// async and reuse it; the sync `RecordsBridge` is the canonical
/// interface the JS runtime calls.
#[async_trait]
pub trait AsyncRecordsBridge: Send + Sync + 'static {
    async fn find_one(&self, collection: &str, id: &str) -> Result<Option<Json>>;
    async fn find_by_filter(
        &self,
        collection: &str,
        filter: &str,
        per_page: u32,
    ) -> Result<Vec<Json>>;
    async fn create(&self, collection: &str, fields: BTreeMap<String, Json>) -> Result<Json>;
    async fn update(
        &self,
        collection: &str,
        id: &str,
        patch: BTreeMap<String, Json>,
    ) -> Result<Json>;
    async fn delete(&self, collection: &str, id: &str) -> Result<()>;
}

/// Wrap an `AsyncRecordsBridge` to satisfy the sync `RecordsBridge`
/// shape that the JS callbacks need. Uses `block_in_place +
/// Handle::block_on`, which is valid because hook dispatch runs on
/// a tokio worker (multi-threaded runtime is required).
pub struct SyncBridge<T: AsyncRecordsBridge>(pub Arc<T>);

impl<T: AsyncRecordsBridge> RecordsBridge for SyncBridge<T> {
    fn find_one(&self, collection: &str, id: &str) -> Result<Option<Json>> {
        let inner = self.0.clone();
        let collection = collection.to_string();
        let id = id.to_string();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(async move { inner.find_one(&collection, &id).await })
        })
    }
    fn find_by_filter(&self, collection: &str, filter: &str, per_page: u32) -> Result<Vec<Json>> {
        let inner = self.0.clone();
        let collection = collection.to_string();
        let filter = filter.to_string();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(async move { inner.find_by_filter(&collection, &filter, per_page).await })
        })
    }
    fn create(&self, collection: &str, fields: BTreeMap<String, Json>) -> Result<Json> {
        let inner = self.0.clone();
        let collection = collection.to_string();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(async move { inner.create(&collection, fields).await })
        })
    }
    fn update(&self, collection: &str, id: &str, patch: BTreeMap<String, Json>) -> Result<Json> {
        let inner = self.0.clone();
        let collection = collection.to_string();
        let id = id.to_string();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(async move { inner.update(&collection, &id, patch).await })
        })
    }
    fn delete(&self, collection: &str, id: &str) -> Result<()> {
        let inner = self.0.clone();
        let collection = collection.to_string();
        let id = id.to_string();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(async move { inner.delete(&collection, &id).await })
        })
    }
}

/// JS host bound to one (realm, app). Multiple `.js` files share its
/// QuickJS context, so they can cooperate via globals.
pub struct AppHooks {
    ctx: AsyncContext,
    #[allow(dead_code)]
    rt: AsyncRuntime,
    records: Option<Arc<dyn RecordsBridge>>,
    mailer: Option<Arc<dyn Mailer>>,
    limits: SandboxLimits,
    clock: CpuClock,
    /// Tokio task handles for `$app.cron` jobs. Aborted on drop so a
    /// reload through `HookEngine::load_app` doesn't leak schedulers.
    cron_tasks: parking_lot::Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

impl Drop for AppHooks {
    fn drop(&mut self) {
        for h in self.cron_tasks.lock().drain(..) {
            h.abort();
        }
    }
}

impl AppHooks {
    pub async fn new() -> Result<Self> {
        Self::with_records(None).await
    }

    pub async fn with_records(records: Option<Arc<dyn RecordsBridge>>) -> Result<Self> {
        Self::with_records_and_limits(records, SandboxLimits::default()).await
    }

    /// Build an `AppHooks` with explicit sandbox limits. Bootstrap JS
    /// (the `$app` global, the records shim) runs *before* the limits
    /// take effect — under aggressive policy a future bootstrap step
    /// might otherwise time out trying to install itself.
    pub async fn with_records_and_limits(
        records: Option<Arc<dyn RecordsBridge>>,
        limits: SandboxLimits,
    ) -> Result<Self> {
        Self::build(records, None, limits).await
    }

    /// Like `with_records_and_limits` but also wires a mailer so user
    /// hooks can call `$app.mailer.send(...)`. The mailer is `None` ⇒
    /// the JS surface is still present but every call throws "no
    /// mailer bound", which keeps the shim shape consistent across
    /// dev (LogMailer) and bare test (no mailer) runs.
    pub async fn with_records_mailer_and_limits(
        records: Option<Arc<dyn RecordsBridge>>,
        mailer: Option<Arc<dyn Mailer>>,
        limits: SandboxLimits,
    ) -> Result<Self> {
        Self::build(records, mailer, limits).await
    }

    async fn build(
        records: Option<Arc<dyn RecordsBridge>>,
        mailer: Option<Arc<dyn Mailer>>,
        limits: SandboxLimits,
    ) -> Result<Self> {
        let rt = AsyncRuntime::new()?;
        let ctx = AsyncContext::full(&rt).await?;
        let clock = CpuClock::new();
        let me = Self {
            rt,
            ctx,
            records,
            mailer,
            limits,
            clock,
            cron_tasks: parking_lot::Mutex::new(Vec::new()),
        };
        me.install_app_global().await?;
        me.apply_limits().await;
        Ok(me)
    }

    /// Apply `self.limits` to the underlying QuickJS runtime. Called
    /// once at the tail of construction, after bootstrap is in.
    async fn apply_limits(&self) {
        if let Some(b) = self.limits.memory_bytes {
            self.rt.set_memory_limit(b).await;
        }
        if let Some(b) = self.limits.stack_bytes {
            self.rt.set_max_stack_size(b).await;
        }
        // The interrupt handler is installed unconditionally — it reads
        // `clock.deadline_ms`, and that field is `0` whenever no entry
        // has armed a deadline, so the handler is a no-op until armed.
        let clock = self.clock.clone();
        self.rt
            .set_interrupt_handler(Some(Box::new(move || clock.deadline_crossed())))
            .await;
    }

    /// If `cpu_time_ms` is configured, arm a deadline guard. Returned
    /// `Option<CpuGuard>` disarms on drop; if no CPU policy is set,
    /// `None` is returned and execution runs uncapped.
    fn arm_cpu(&self) -> Option<sandbox::CpuGuard<'_>> {
        self.limits.cpu_time_ms.map(|ms| self.clock.arm(ms))
    }

    /// Convert a generic JS error into `Timeout` when the CPU deadline
    /// was crossed during the call. Otherwise pass the error through.
    fn classify(&self, e: RuntimeError) -> RuntimeError {
        match (&e, self.limits.cpu_time_ms, self.clock.deadline_crossed()) {
            (RuntimeError::Js(_), Some(ms), true) => RuntimeError::Timeout(ms),
            _ => e,
        }
    }

    /// Evaluate a single JS source. Errors are returned for visibility
    /// (the caller logs them); a failing file does not poison the
    /// context — later hooks may still load and run. CPU deadline is
    /// armed for the duration of the call.
    pub async fn eval(&self, src: &str, label: &str) -> Result<()> {
        let src = src.to_string();
        let label = label.to_string();
        let _cpu = self.arm_cpu();
        let res = self
            .ctx
            .with(move |ctx| {
                let _: rquickjs::Value = ctx
                    .eval(src.as_bytes())
                    .catch(&ctx)
                    .map_err(|e| RuntimeError::Js(format!("{label}: {e}")))?;
                Ok::<_, RuntimeError>(())
            })
            .await;
        res.map_err(|e| self.classify(e))
    }

    /// Walk `dir`, evaluating every `*.js` file. Failing files are
    /// logged but don't abort the load — other hooks may still be
    /// valid. Returns the number of files successfully evaluated.
    pub async fn load_dir(&self, dir: &Path) -> Result<usize> {
        if !dir.is_dir() {
            return Ok(0);
        }
        let mut loaded = 0usize;
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            let ext = path.extension().and_then(|s| s.to_str());
            let is_ts = ext == Some("ts");
            let is_js = ext == Some("js");
            if !is_ts && !is_js {
                continue;
            }
            let label = path.display().to_string();
            let raw = std::fs::read_to_string(&path)?;
            // .ts files run through swc's TypeScript strip first; the
            // emitted JS is what the QuickJS context evaluates.
            let src = if is_ts {
                match ts::transpile(&raw) {
                    Ok(js) => js,
                    Err(e) => {
                        tracing::error!(file = %label, error = %e, "ts transpile failed");
                        continue;
                    }
                }
            } else {
                raw
            };
            if let Err(e) = self.eval(&src, &label).await {
                tracing::error!(file = %label, error = %e, "hook load failed");
            } else {
                loaded += 1;
                tracing::info!(file = %label, "hook loaded");
            }
        }
        Ok(loaded)
    }

    /// Invoke every handler registered for `(collection, event)`.
    /// `payload` is serialised once and inlined as a JSON-literal
    /// string into the dispatch driver.
    pub async fn dispatch<T: Serialize>(
        &self,
        collection: &str,
        event: HookEvent,
        request: &HookRequest,
        payload: &T,
    ) -> Result<()> {
        let key = format!("{}:{}", collection, event.as_str());
        let json = serde_json::to_string(payload)
            .map_err(|e| RuntimeError::Js(format!("serialise payload: {e}")))?;
        let request_json = serde_json::to_string(request)
            .map_err(|e| RuntimeError::Js(format!("serialise request: {e}")))?;
        // `$app.request` is set for the duration of dispatch and
        // nulled in finally so leftover state doesn't leak into the
        // next call.
        let driver = format!(
            r#"
            (function() {{
                globalThis.$app.request = JSON.parse({request_lit});
                try {{
                    const list = (globalThis.__rb_handlers || {{}})[{key}] || [];
                    const payload = JSON.parse({payload_lit});
                    for (const fn of list) {{
                        try {{ fn(payload); }}
                        catch (e) {{ globalThis.__rb_record_error(String(e)); }}
                    }}
                }} finally {{
                    globalThis.$app.request = null;
                }}
            }})();
            "#,
            key = json_quote(&key),
            payload_lit = json_quote(&json),
            request_lit = json_quote(&request_json),
        );
        let _cpu = self.arm_cpu();
        let res = self
            .ctx
            .with(move |ctx| {
                let _: rquickjs::Value = ctx
                    .eval(driver.as_bytes())
                    .catch(&ctx)
                    .map_err(|e| RuntimeError::Js(format!("dispatch: {e}")))?;
                Ok::<_, RuntimeError>(())
            })
            .await;
        res.map_err(|e| self.classify(e))?;
        Ok(())
    }

    /// Run BEFORE-create hooks against `payload` (the incoming
    /// fields). Handlers may mutate the object or throw. Returns the
    /// (possibly mutated) payload on success, or `Err(Veto)` if any
    /// handler threw — the caller maps that to a 4xx and skips the
    /// DB write.
    pub async fn dispatch_before_create(
        &self,
        collection: &str,
        request: &HookRequest,
        payload: BTreeMap<String, Json>,
    ) -> Result<BTreeMap<String, Json>> {
        let result = self
            .run_before(
                &format!("{collection}:before_create"),
                request,
                &serde_json::to_string(&payload)
                    .map_err(|e| RuntimeError::Js(format!("serialise: {e}")))?,
                "fn(payload)",
                "payload",
                None,
            )
            .await?;
        serde_json::from_str(&result)
            .map_err(|e| RuntimeError::Js(format!("deserialise mutated payload: {e}")))
    }

    /// Run BEFORE-update hooks with `(existing, patch)`. `patch` is
    /// the only object handlers should mutate; `existing` is a
    /// snapshot of the row prior to the write. Returns the mutated
    /// patch on success.
    pub async fn dispatch_before_update<E: Serialize>(
        &self,
        collection: &str,
        request: &HookRequest,
        existing: &E,
        patch: BTreeMap<String, Json>,
    ) -> Result<BTreeMap<String, Json>> {
        let existing_json = serde_json::to_string(existing)
            .map_err(|e| RuntimeError::Js(format!("serialise existing: {e}")))?;
        let patch_json = serde_json::to_string(&patch)
            .map_err(|e| RuntimeError::Js(format!("serialise patch: {e}")))?;
        let result = self
            .run_before(
                &format!("{collection}:before_update"),
                request,
                &patch_json,
                "fn(existing, patch)",
                "patch",
                Some(("existing", &existing_json)),
            )
            .await?;
        serde_json::from_str(&result)
            .map_err(|e| RuntimeError::Js(format!("deserialise mutated patch: {e}")))
    }

    /// Run BEFORE-delete hooks against `existing`. Returns `Ok(())`
    /// on success; `Err(Veto)` if any handler threw.
    pub async fn dispatch_before_delete<E: Serialize>(
        &self,
        collection: &str,
        request: &HookRequest,
        existing: &E,
    ) -> Result<()> {
        let existing_json = serde_json::to_string(existing)
            .map_err(|e| RuntimeError::Js(format!("serialise existing: {e}")))?;
        let _ = self
            .run_before(
                &format!("{collection}:before_delete"),
                request,
                &existing_json,
                "fn(existing)",
                "existing",
                None,
            )
            .await?;
        Ok(())
    }

    /// Run a "before-*" handler set keyed by the user-lifecycle event
    /// against a JSON payload (the user). No mutation propagates back;
    /// the return value of run_before is discarded. Returns `Err(Veto)`
    /// if any handler threw, otherwise `Ok(())`.
    pub async fn dispatch_before_user_event<U: Serialize>(
        &self,
        event: HookEvent,
        request: &HookRequest,
        user: &U,
    ) -> Result<()> {
        let user_json = serde_json::to_string(user)
            .map_err(|e| RuntimeError::Js(format!("serialise user: {e}")))?;
        let _ = self
            .run_before(
                &format!("{}:{}", USER_HOOK_COLLECTION, event.as_str()),
                request,
                &user_json,
                "fn(user)",
                "user",
                None,
            )
            .await?;
        Ok(())
    }

    /// Look up + invoke a custom HTTP route registered via
    /// `$app.routerAdd(method, path, fn)`. Returns `Ok(None)` when no
    /// handler is registered for `(method, path)` so the API layer
    /// can answer 404. Returns `Ok(Some(_))` with the JSON response
    /// shape the handler produced. The CPU deadline is armed for the
    /// duration of the call, same as for record/lifecycle dispatch.
    pub async fn invoke_custom_route(
        &self,
        method: &str,
        path: &str,
        ctx: &CustomRouteContext,
    ) -> Result<Option<CustomRouteResponse>> {
        let ctx_json = serde_json::to_string(ctx)
            .map_err(|e| RuntimeError::Js(format!("serialise ctx: {e}")))?;
        let driver = format!(
            r#"
            (function() {{
                return globalThis.__rb_invoke_route({method_lit}, {path_lit}, {ctx_lit});
            }})();
            "#,
            method_lit = json_quote(method),
            path_lit = json_quote(path),
            ctx_lit = json_quote(&ctx_json),
        );
        let _cpu = self.arm_cpu();
        let raw: String = self
            .ctx
            .with(move |ctx| {
                let v: String = ctx
                    .eval(driver.as_bytes())
                    .catch(&ctx)
                    .map_err(|e| RuntimeError::Js(format!("invoke_route: {e}")))?;
                Ok::<_, RuntimeError>(v)
            })
            .await
            .map_err(|e| self.classify(e))?;
        if raw.is_empty() {
            return Ok(None);
        }
        let resp: CustomRouteResponse = serde_json::from_str(&raw)
            .map_err(|e| RuntimeError::Js(format!("invoke_route response: {e}")))?;
        Ok(Some(resp))
    }

    /// Invoke the cron handler registered with `id`. Used by the
    /// scheduler tasks spawned in `start_cron_tasks` and by tests
    /// that prefer driving the JS dispatch path directly rather than
    /// waiting for a real tick to fire.
    pub async fn invoke_cron(&self, id: u64) -> Result<()> {
        let driver = format!("(function() {{ globalThis.__rb_invoke_cron({id}); }})();");
        let _cpu = self.arm_cpu();
        self.ctx
            .with(move |ctx| {
                let _: rquickjs::Value = ctx
                    .eval(driver.as_bytes())
                    .catch(&ctx)
                    .map_err(|e| RuntimeError::Js(format!("invoke_cron: {e}")))?;
                Ok::<_, RuntimeError>(())
            })
            .await
            .map_err(|e| self.classify(e))?;
        Ok(())
    }

    /// Drain `__rb_pending_crons` from the JS side, parse each cron
    /// expression, and spawn one tokio task per job that ticks on the
    /// schedule and calls `invoke_cron(id)` on every fire. A bad
    /// expression is logged and skipped — other valid jobs in the
    /// same hook file still spin up.
    ///
    /// Returns the number of tasks spawned.
    ///
    /// Idempotent: re-calling this after a no-op load (empty pending
    /// list) is a no-op. Drop on `AppHooks` aborts every spawned task.
    pub async fn start_cron_tasks(self: &Arc<Self>) -> Result<usize> {
        // Pull the queue out of JS land in one shot, then clear it so
        // a subsequent load_dir doesn't double-schedule what we just
        // consumed.
        let pending_json: String = self
            .ctx
            .with(|ctx| {
                let v: String = ctx
                    .eval(
                        r#"
                        (function() {
                            const out = JSON.stringify(globalThis.__rb_pending_crons || []);
                            globalThis.__rb_pending_crons = [];
                            return out;
                        })();
                        "#
                        .as_bytes(),
                    )
                    .catch(&ctx)
                    .map_err(|e| RuntimeError::Js(format!("drain pending crons: {e}")))?;
                Ok::<_, RuntimeError>(v)
            })
            .await?;

        #[derive(Deserialize)]
        struct PendingCron {
            id: u64,
            expr: String,
        }
        let pending: Vec<PendingCron> = serde_json::from_str(&pending_json)
            .map_err(|e| RuntimeError::Js(format!("decode pending crons: {e}")))?;

        let mut started = 0usize;
        for job in pending {
            let schedule = match <cron::Schedule as std::str::FromStr>::from_str(&job.expr) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(
                        cron = %job.expr,
                        error = %e,
                        "skipping invalid cron expression"
                    );
                    continue;
                }
            };
            let weak: std::sync::Weak<AppHooks> = Arc::downgrade(self);
            let expr = job.expr.clone();
            let id = job.id;
            let handle = tokio::spawn(async move {
                let mut sched_iter = schedule.upcoming(chrono::Utc);
                loop {
                    // Advance to the next fire time after "now".
                    let next = sched_iter.next();
                    let Some(next) = next else {
                        // Schedule exhausted (one-shot-ish expressions
                        // hit this); we're done with this job.
                        break;
                    };
                    let now = chrono::Utc::now();
                    let dur = (next - now).to_std().unwrap_or_default();
                    tokio::time::sleep(dur).await;

                    // Upgrade the weak ref. If AppHooks was dropped
                    // (e.g. hook reload), exit cleanly.
                    let Some(hooks) = weak.upgrade() else {
                        return;
                    };
                    if let Err(e) = hooks.invoke_cron(id).await {
                        tracing::warn!(
                            cron = %expr,
                            error = %e,
                            "cron handler errored"
                        );
                    }
                }
            });
            self.cron_tasks.lock().push(handle);
            started += 1;
        }
        Ok(started)
    }

    /// Shared driver for before-* dispatch. Builds a JS IIFE that
    /// iterates handlers, calling each with the right shape. On a
    /// caught throw, the driver returns the sentinel string
    /// `"__rb_veto:<msg>"`. On success, it returns
    /// `JSON.stringify(<primary_arg>)` so we can pick up mutations.
    async fn run_before(
        &self,
        key: &str,
        request: &HookRequest,
        primary_arg_json: &str,
        // human-readable shape, currently unused except for clarity
        _signature: &str,
        // which JS binding is the "mutable primary" we re-stringify
        primary_name: &str,
        secondary: Option<(&str, &str)>,
    ) -> Result<String> {
        let (sec_decl, call_args) = match secondary {
            Some((name, json)) => (
                format!("const {name} = JSON.parse({});", json_quote(json)),
                format!("{name}, {primary_name}"),
            ),
            None => (String::new(), primary_name.to_string()),
        };
        let request_json = serde_json::to_string(request)
            .map_err(|e| RuntimeError::Js(format!("serialise request: {e}")))?;
        let driver = format!(
            r#"
            (function() {{
                globalThis.$app.request = JSON.parse({request_lit});
                try {{
                    const list = (globalThis.__rb_handlers || {{}})[{key_lit}] || [];
                    const {primary} = JSON.parse({payload_lit});
                    {sec_decl}
                    for (const fn of list) {{
                        try {{ fn({call_args}); }}
                        catch (e) {{
                            return "__rb_veto:" + String((e && e.message) || e);
                        }}
                    }}
                    return JSON.stringify({primary});
                }} finally {{
                    globalThis.$app.request = null;
                }}
            }})();
            "#,
            key_lit = json_quote(key),
            primary = primary_name,
            payload_lit = json_quote(primary_arg_json),
            sec_decl = sec_decl,
            call_args = call_args,
            request_lit = json_quote(&request_json),
        );

        let _cpu = self.arm_cpu();
        let result: String = self
            .ctx
            .with(move |ctx| {
                let v: String = ctx
                    .eval(driver.as_bytes())
                    .catch(&ctx)
                    .map_err(|e| RuntimeError::Js(format!("dispatch_before: {e}")))?;
                Ok::<_, RuntimeError>(v)
            })
            .await
            .map_err(|e| self.classify(e))?;

        if let Some(msg) = result.strip_prefix("__rb_veto:") {
            return Err(RuntimeError::Veto(msg.to_string()));
        }
        Ok(result)
    }

    async fn install_app_global(&self) -> Result<()> {
        // Pure-JS half: handler registry, error collector, $app.log
        // that ALSO stashes to __rb_log so tests can drain it.
        const BOOTSTRAP_JS: &str = r#"
            (function() {
                const handlers = (globalThis.__rb_handlers = {});
                const errors = (globalThis.__rb_errors = []);
                globalThis.__rb_log = [];
                // Custom HTTP routes registered via $app.routerAdd.
                // Keyed by "METHOD /path". One handler per key (a
                // re-register replaces silently — matches axum's
                // last-write-wins for duplicate routes).
                globalThis.__rb_routes = {};
                // Scheduled jobs registered via $app.cron. The JS
                // half is registry + dispatch only; actual scheduling
                // is Rust-side and starts after hook load completes.
                //   __rb_cron_jobs[id]   = handler fn
                //   __rb_pending_crons   = [{id, expr}, ...]
                //                          (drained by start_cron_tasks)
                //   __rb_cron_next_id    = monotonic sequence
                globalThis.__rb_cron_jobs = {};
                globalThis.__rb_pending_crons = [];
                globalThis.__rb_cron_next_id = 0;
                globalThis.__rb_record_error = function(msg) { errors.push(String(msg)); };

                // Invoked by the Rust scheduler at each cron tick.
                // Errors are caught and routed to __rb_errors so a
                // broken job doesn't kill its peers.
                globalThis.__rb_invoke_cron = function(id) {
                    const fn = globalThis.__rb_cron_jobs[id];
                    if (typeof fn !== 'function') return '';
                    try { fn(); }
                    catch (e) { globalThis.__rb_record_error(String(e)); }
                    return '';
                };

                function register(kind, collection, fn) {
                    if (typeof fn !== 'function') {
                        throw new Error('handler must be a function');
                    }
                    const key = collection + ':' + kind;
                    (handlers[key] = handlers[key] || []).push(fn);
                }

                // Invoked by the Rust catch-all when a custom-route
                // request arrives. Returns a JSON string describing
                // the response, or the empty string when no handler
                // is registered for (method, path).
                globalThis.__rb_invoke_route = function(method, path, ctxJson) {
                    const key = method.toUpperCase() + ' ' + path;
                    const fn = globalThis.__rb_routes[key];
                    if (typeof fn !== 'function') return '';
                    const ctx = JSON.parse(ctxJson);
                    let res;
                    try { res = fn(ctx); }
                    catch (e) {
                        globalThis.__rb_record_error(String(e));
                        return JSON.stringify({
                            status: 500,
                            body: { error: String((e && e.message) || e) },
                        });
                    }
                    if (res === undefined || res === null) {
                        return JSON.stringify({ status: 204 });
                    }
                    if (typeof res !== 'object') {
                        return JSON.stringify({ status: 200, body: res });
                    }
                    if (typeof res.status !== 'number') res.status = 200;
                    return JSON.stringify(res);
                };

                globalThis.$app = {
                    // populated per-call by the dispatch driver; null
                    // outside any dispatch (e.g. at hook-load time).
                    request: null,
                    onRecordAfterCreate(collection, fn) { register('after_create', collection, fn); },
                    onRecordAfterUpdate(collection, fn) { register('after_update', collection, fn); },
                    onRecordAfterDelete(collection, fn) { register('after_delete', collection, fn); },
                    onRecordBeforeCreate(collection, fn) { register('before_create', collection, fn); },
                    onRecordBeforeUpdate(collection, fn) { register('before_update', collection, fn); },
                    onRecordBeforeDelete(collection, fn) { register('before_delete', collection, fn); },
                    // Realm-wide user-lifecycle hooks. No collection
                    // argument; they fire on every authentication
                    // track (password / OTP / OAuth) and signup path.
                    // The handler receives `(user)` where user is
                    // `{id, email, verified}` — never the password
                    // hash. before_login can throw to veto.
                    onUserBeforeLogin(fn)   { register('user_before_login',   '_user', fn); },
                    onUserAfterLogin(fn)    { register('user_after_login',    '_user', fn); },
                    onUserAfterRegister(fn) { register('user_after_register', '_user', fn); },
                    // Per-app mailer-lifecycle hooks. Fire only on
                    // $app.mailer.send invocations from THIS app
                    // (the QuotedMailer / SmtpMailer system path for
                    // verify-email / password-reset / OTP is not
                    // intercepted — those are server-issued mail).
                    // before_send can throw to veto the send;
                    // after_send is an observer (errors stashed).
                    onMailerBeforeSend(fn) { register('mailer_before_send', '_mail', fn); },
                    onMailerAfterSend(fn)  { register('mailer_after_send',  '_mail', fn); },
                    // Custom HTTP endpoints. Mounted under
                    //   /api/realms/<realm>/apps/<app>/custom<path>
                    // so routerAdd("GET", "/hello", fn) becomes
                    //   GET /api/realms/<realm>/apps/<app>/custom/hello.
                    //
                    // The handler receives an object with these fields:
                    //   method   uppercased verb (string)
                    //   path     the path it matched (no prefix)
                    //   query    query string parsed as { [k]: string }
                    //   headers  request header map { [k]: string }
                    //   body     parsed JSON body or null on
                    //            non-JSON / empty bodies
                    //
                    // Return shape:
                    //   { status: 200, body: any, headers?: object }
                    // Missing return / undefined -> 204 No Content.
                    // Non-object return -> 200 with that value as body.
                    // Throw -> 500 Internal Server Error; the error
                    //          message is logged via __rb_record_error.
                    //
                    // Phase 1: exact path match. No `:param` or wildcard.
                    routerAdd(method, path, fn) {
                        if (typeof method !== 'string') {
                            throw new Error('routerAdd: method must be a string');
                        }
                        if (typeof path !== 'string' || !path.startsWith('/')) {
                            throw new Error('routerAdd: path must start with "/"');
                        }
                        if (typeof fn !== 'function') {
                            throw new Error('routerAdd: handler must be a function');
                        }
                        globalThis.__rb_routes[method.toUpperCase() + ' ' + path] = fn;
                    },
                    // Schedule a handler against a cron expression.
                    // The expression follows the `cron` crate's
                    // 6-field shape (sec min hour dom mon dow) — see
                    // https://docs.rs/cron for the grammar. Registration
                    // is pure-JS at hook-load time; the Rust scheduler
                    // picks up __rb_pending_crons once hook load
                    // completes, parses each expression, and spawns one
                    // tokio task per job. A bad expression aborts at
                    // scheduler-start time (visible in the log), not
                    // here, so all valid jobs in the same hook file
                    // still register.
                    //
                    //   $app.cron("0 0 * * * *", () => $app.log("hourly"));
                    cron(expr, fn) {
                        if (typeof expr !== 'string') {
                            throw new Error('cron: expression must be a string');
                        }
                        if (typeof fn !== 'function') {
                            throw new Error('cron: handler must be a function');
                        }
                        const id = ++globalThis.__rb_cron_next_id;
                        globalThis.__rb_cron_jobs[id] = fn;
                        globalThis.__rb_pending_crons.push({ id, expr });
                        return id;
                    },
                    log(msg) {
                        const s = String(msg);
                        globalThis.__rb_log.push(s);
                        if (typeof globalThis.__rb_native_log === 'function') {
                            try { globalThis.__rb_native_log(s); } catch (e) { /* ignore */ }
                        }
                    },
                };
            })();
        "#;
        self.eval(BOOTSTRAP_JS, "<bootstrap>").await?;

        // Rust half: bind __rb_native_log → tracing so $app.log() shows
        // up in the server's logs. Test-only callers can still drain
        // the pure-JS __rb_log array.
        let records = self.records.clone();
        let mailer = self.mailer.clone();
        self.ctx
            .with(move |ctx| {
                use rquickjs::Function;

                let log_fn = Function::new(ctx.clone(), |msg: String| {
                    tracing::info!(target: "rustbase_runtime::hook", "{msg}");
                })?
                .with_name("__rb_native_log")?;
                ctx.globals().set("__rb_native_log", log_fn)?;

                // $app.records.* bindings — only wired if the engine
                // was constructed with a bridge. Tests + standalone
                // engine instances get a stub that throws.
                if let Some(bridge) = records {
                    register_records_natives(ctx.clone(), bridge)?;
                }

                // $app.mailer.send binding. Without a mailer the shim
                // throws "no mailer bound" — same shape as records.
                if let Some(m) = mailer {
                    register_mailer_native(ctx.clone(), m)?;
                }

                Ok::<_, rquickjs::Error>(())
            })
            .await?;

        // JS shim wrapping the natives in JSON-typed accessors.
        const RECORDS_SHIM: &str = r#"
            (function() {
                const ERR = '__rb_err:';
                function call(name, args) {
                    if (typeof globalThis[name] !== 'function') {
                        throw new Error(name + " is not available (no bridge bound)");
                    }
                    const s = globalThis[name].apply(null, args);
                    if (typeof s === 'string' && s.indexOf(ERR) === 0) {
                        throw new Error(s.slice(ERR.length));
                    }
                    return s;
                }
                $app.records = {
                    findOne(collection, id) {
                        const s = call('__rb_records_findOne', [collection, id]);
                        return s === 'null' ? null : JSON.parse(s);
                    },
                    findByFilter(collection, filter, perPage) {
                        const s = call('__rb_records_findByFilter', [collection, filter, perPage || 30]);
                        return JSON.parse(s);
                    },
                    create(collection, fields) {
                        const s = call('__rb_records_create', [collection, JSON.stringify(fields || {})]);
                        return JSON.parse(s);
                    },
                    update(collection, id, patch) {
                        const s = call('__rb_records_update', [collection, id, JSON.stringify(patch || {})]);
                        return JSON.parse(s);
                    },
                    delete(collection, id) {
                        call('__rb_records_delete', [collection, id]);
                    },
                };
                $app.mailer = {
                    /**
                     * Send one outbound email synchronously from a hook.
                     *
                     *   $app.mailer.send({
                     *     from: "no-reply@app.com",
                     *     to:   "alice@example.com",
                     *     subject: "hi",
                     *     text: "plain body",
                     *     html: "<p>optional html body</p>",  // optional
                     *   });
                     *
                     * Lifecycle:
                     *   1. Every registered `onMailerBeforeSend(fn)` fires
                     *      with the message. Any throw aborts the send;
                     *      the throw propagates to this call's caller.
                     *   2. Native transport (QuotedMailer → SmtpMailer
                     *      etc) actually delivers.
                     *   3. Every registered `onMailerAfterSend(fn)` fires.
                     *      Throws from after-send are caught and stashed
                     *      in `__rb_errors`; they don't roll back the
                     *      already-completed delivery.
                     *
                     * Throws if no mailer is bound (test contexts) or if
                     * the underlying transport rejects the message.
                     */
                    send(msg) {
                        if (!msg || typeof msg !== 'object') {
                            throw new Error('$app.mailer.send: message must be an object');
                        }
                        const before = (globalThis.__rb_handlers || {})['_mail:mailer_before_send'] || [];
                        for (const fn of before) {
                            // Throw propagates → veto. We deliberately
                            // do NOT swallow here; aborting the send is
                            // the whole point of the before-send hook.
                            fn(msg);
                        }
                        call('__rb_mailer_send', [JSON.stringify(msg)]);
                        const after = (globalThis.__rb_handlers || {})['_mail:mailer_after_send'] || [];
                        for (const fn of after) {
                            try { fn(msg); }
                            catch (e) { globalThis.__rb_record_error(String(e)); }
                        }
                    },
                };
            })();
        "#;
        self.eval(RECORDS_SHIM, "<records-shim>").await?;

        Ok(())
    }

    /// Drain any `$app.log(...)` messages emitted since the last call.
    pub async fn drain_logs(&self) -> Result<Vec<String>> {
        let logs = self
            .ctx
            .with(|ctx| {
                let logs: Vec<String> = ctx
                    .eval(
                        r#"
                        (function() {
                            const out = globalThis.__rb_log || [];
                            globalThis.__rb_log = [];
                            return out;
                        })();
                        "#
                        .as_bytes(),
                    )
                    .catch(&ctx)
                    .map_err(|e| RuntimeError::Js(format!("drain logs: {e}")))?;
                Ok::<_, RuntimeError>(logs)
            })
            .await?;
        Ok(logs)
    }

    /// Drain any errors raised by handlers (caught via the dispatch
    /// try/catch). Useful for tests and structured logging.
    pub async fn drain_errors(&self) -> Result<Vec<String>> {
        let errs = self
            .ctx
            .with(|ctx| {
                let out: Vec<String> = ctx
                    .eval(
                        r#"
                        (function() {
                            const out = globalThis.__rb_errors || [];
                            globalThis.__rb_errors = [];
                            return out;
                        })();
                        "#
                        .as_bytes(),
                    )
                    .catch(&ctx)
                    .map_err(|e| RuntimeError::Js(format!("drain errors: {e}")))?;
                Ok::<_, RuntimeError>(out)
            })
            .await?;
        Ok(errs)
    }
}

/// Encode `s` as a JS string literal (with surrounding quotes).
fn json_quote(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// Bind `__rb_records_*` natives that the JS shim calls into. Each
/// returns a JSON string (or throws a JS error); the shim does the
/// `JSON.parse`. Strings instead of typed rquickjs values keeps the
/// FFI small and avoids spelling out conversions for nested
/// structures.
fn register_records_natives(
    ctx: rquickjs::Ctx<'_>,
    bridge: Arc<dyn RecordsBridge>,
) -> std::result::Result<(), rquickjs::Error> {
    use rquickjs::Function;

    // Each native returns a JSON-encoded string. To signal an error
    // without spelling out rquickjs's Exception machinery (which
    // differs between versions), we return a string with a sentinel
    // `__rb_err:` prefix; the JS shim detects this and throws.
    const ERR: &str = "__rb_err:";

    let b1 = bridge.clone();
    let find_one = Function::new(
        ctx.clone(),
        move |collection: String, id: String| -> String {
            match b1.find_one(&collection, &id) {
                Ok(Some(rec)) => serde_json::to_string(&rec).unwrap_or_else(|_| "null".into()),
                Ok(None) => "null".to_string(),
                Err(e) => format!("{ERR}{e}"),
            }
        },
    )?
    .with_name("__rb_records_findOne")?;
    ctx.globals().set("__rb_records_findOne", find_one)?;

    let b2 = bridge.clone();
    let find_by_filter = Function::new(
        ctx.clone(),
        move |collection: String, filter: String, per_page: u32| -> String {
            match b2.find_by_filter(&collection, &filter, per_page) {
                Ok(rows) => serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into()),
                Err(e) => format!("{ERR}{e}"),
            }
        },
    )?
    .with_name("__rb_records_findByFilter")?;
    ctx.globals()
        .set("__rb_records_findByFilter", find_by_filter)?;

    let b3 = bridge.clone();
    let create = Function::new(
        ctx.clone(),
        move |collection: String, fields_json: String| -> String {
            let fields: BTreeMap<String, Json> = match serde_json::from_str(&fields_json) {
                Ok(v) => v,
                Err(e) => return format!("{ERR}fields: {e}"),
            };
            match b3.create(&collection, fields) {
                Ok(rec) => serde_json::to_string(&rec).unwrap_or_else(|_| "{}".into()),
                Err(e) => format!("{ERR}{e}"),
            }
        },
    )?
    .with_name("__rb_records_create")?;
    ctx.globals().set("__rb_records_create", create)?;

    let b4 = bridge.clone();
    let update = Function::new(
        ctx.clone(),
        move |collection: String, id: String, patch_json: String| -> String {
            let patch: BTreeMap<String, Json> = match serde_json::from_str(&patch_json) {
                Ok(v) => v,
                Err(e) => return format!("{ERR}patch: {e}"),
            };
            match b4.update(&collection, &id, patch) {
                Ok(rec) => serde_json::to_string(&rec).unwrap_or_else(|_| "{}".into()),
                Err(e) => format!("{ERR}{e}"),
            }
        },
    )?
    .with_name("__rb_records_update")?;
    ctx.globals().set("__rb_records_update", update)?;

    let b5 = bridge;
    let delete_fn = Function::new(
        ctx.clone(),
        move |collection: String, id: String| -> String {
            match b5.delete(&collection, &id) {
                Ok(()) => "null".to_string(),
                Err(e) => format!("{ERR}{e}"),
            }
        },
    )?
    .with_name("__rb_records_delete")?;
    ctx.globals().set("__rb_records_delete", delete_fn)?;

    Ok(())
}

/// JSON shape the JS shim hands us for `$app.mailer.send(msg)`.
#[derive(Deserialize)]
struct MailerSendArgs {
    from: String,
    to: String,
    subject: String,
    text: String,
    #[serde(default)]
    html: Option<String>,
}

/// Bind `__rb_mailer_send` so the JS shim's `$app.mailer.send(msg)`
/// reaches `Arc<dyn Mailer>`. The JS side serialises the message
/// object; this side parses, dispatches the async `send` on the
/// current tokio runtime via `block_in_place + block_on`, and
/// returns `"null"` on success or `__rb_err:<reason>` on failure.
fn register_mailer_native(
    ctx: rquickjs::Ctx<'_>,
    mailer: Arc<dyn Mailer>,
) -> std::result::Result<(), rquickjs::Error> {
    use rquickjs::Function;
    const ERR: &str = "__rb_err:";

    let send_fn = Function::new(ctx.clone(), move |msg_json: String| -> String {
        let parsed: MailerSendArgs = match serde_json::from_str(&msg_json) {
            Ok(v) => v,
            Err(e) => return format!("{ERR}invalid message: {e}"),
        };
        let mut msg = EmailMessage::new(parsed.from, parsed.to, parsed.subject, parsed.text);
        if let Some(h) = parsed.html {
            msg = msg.with_html(h);
        }
        let mailer = mailer.clone();
        // Hooks run on a tokio worker; block_in_place + block_on is the
        // sanctioned way to call async code from a sync FFI shim.
        let res = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move { mailer.send(msg).await })
        });
        match res {
            Ok(()) => "null".to_string(),
            Err(e) => format!("{ERR}{e}"),
        }
    })?
    .with_name("__rb_mailer_send")?;
    ctx.globals().set("__rb_mailer_send", send_fn)?;
    Ok(())
}

/// Engine bound to the whole server. Holds one `AppHooks` per
/// (realm, app) — lazily created on first hook load. `Clone` is
/// cheap; the inner map is `Arc`'d.
#[derive(Clone, Default)]
pub struct HookEngine {
    apps: Arc<DashMap<(String, String), Arc<AppHooks>>>,
}

impl HookEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load (or reload) hooks for `(realm, app)` from `dir`. Existing
    /// state for this app is discarded — a fresh `AppHooks` is built
    /// and any previously-registered handlers vanish.
    ///
    /// `bridge` exposes `$app.records.*` to user code; `mailer`
    /// exposes `$app.mailer.send(...)`. Either can be `None` — the
    /// matching JS surface throws "no bridge bound" when invoked.
    pub async fn load_app(
        &self,
        realm: &str,
        app: &str,
        dir: &Path,
        bridge: Option<Arc<dyn RecordsBridge>>,
        mailer: Option<Arc<dyn Mailer>>,
    ) -> Result<usize> {
        let hooks =
            AppHooks::with_records_mailer_and_limits(bridge, mailer, SandboxLimits::default())
                .await?;
        let hooks = Arc::new(hooks);
        let loaded = hooks.load_dir(dir).await?;
        // Spawn schedulers for any $app.cron registrations the hook
        // files populated. start_cron_tasks needs `&Arc<Self>` so it
        // can hand each task a Weak<AppHooks> for clean shutdown on
        // reload.
        if let Err(e) = hooks.start_cron_tasks().await {
            tracing::warn!(realm = %realm, app = %app, error = %e, "starting cron tasks failed");
        }
        self.apps
            .insert((realm.to_string(), app.to_string()), hooks);
        Ok(loaded)
    }

    /// Look up the AppHooks for `(realm, app)`. Returns `None` if no
    /// hooks were ever loaded for that app — dispatch then becomes a
    /// no-op.
    pub fn get(&self, realm: &str, app: &str) -> Option<Arc<AppHooks>> {
        self.apps
            .get(&(realm.to_string(), app.to_string()))
            .map(|h| h.clone())
    }

    /// Dispatch an event. No-op when no hooks are loaded for the
    /// (realm, app). Hook errors are caught by the dispatch driver
    /// and stashed in `__rb_errors`; this method itself succeeds even
    /// when a handler threw.
    pub async fn dispatch<T: Serialize>(
        &self,
        realm: &str,
        app: &str,
        collection: &str,
        event: HookEvent,
        request: &HookRequest,
        payload: &T,
    ) -> Result<()> {
        let Some(hooks) = self.get(realm, app) else {
            return Ok(());
        };
        hooks.dispatch(collection, event, request, payload).await
    }

    /// Before-create. If no hooks are loaded, returns `payload`
    /// unchanged. `Err(Veto)` is propagated to the API layer as 400.
    pub async fn dispatch_before_create(
        &self,
        realm: &str,
        app: &str,
        collection: &str,
        request: &HookRequest,
        payload: BTreeMap<String, Json>,
    ) -> Result<BTreeMap<String, Json>> {
        let Some(hooks) = self.get(realm, app) else {
            return Ok(payload);
        };
        hooks
            .dispatch_before_create(collection, request, payload)
            .await
    }

    pub async fn dispatch_before_update<E: Serialize>(
        &self,
        realm: &str,
        app: &str,
        collection: &str,
        request: &HookRequest,
        existing: &E,
        patch: BTreeMap<String, Json>,
    ) -> Result<BTreeMap<String, Json>> {
        let Some(hooks) = self.get(realm, app) else {
            return Ok(patch);
        };
        hooks
            .dispatch_before_update(collection, request, existing, patch)
            .await
    }

    pub async fn dispatch_before_delete<E: Serialize>(
        &self,
        realm: &str,
        app: &str,
        collection: &str,
        request: &HookRequest,
        existing: &E,
    ) -> Result<()> {
        let Some(hooks) = self.get(realm, app) else {
            return Ok(());
        };
        hooks
            .dispatch_before_delete(collection, request, existing)
            .await
    }

    /// Iterator over `(realm, app, AppHooks)` for every app whose
    /// hooks have been loaded under `realm`. Used by the user-lifecycle
    /// fan-out below.
    fn apps_in_realm(&self, realm: &str) -> Vec<Arc<AppHooks>> {
        self.apps
            .iter()
            .filter_map(|e| {
                let ((r, _a), hooks) = (e.key(), e.value());
                (r == realm).then(|| hooks.clone())
            })
            .collect()
    }

    /// Fire `onUserBeforeLogin` across every app's hooks in the realm.
    /// Vetoable: if any app's hook throws, return `Err(Veto)` and the
    /// caller should abort the login. Apps that aren't loaded yet
    /// contribute no veto.
    pub async fn dispatch_user_before_login<U: Serialize>(
        &self,
        realm: &str,
        request: &HookRequest,
        user: &U,
    ) -> Result<()> {
        for hooks in self.apps_in_realm(realm) {
            hooks
                .dispatch_before_user_event(HookEvent::UserBeforeLogin, request, user)
                .await?;
        }
        Ok(())
    }

    /// Observer fan-out: every app's `onUserAfterLogin` fires.
    /// Per-app handler errors are caught by the dispatch driver and
    /// never bubble; this method only errors if a JS context itself
    /// blew up (which would have surfaced at load time).
    pub async fn dispatch_user_after_login<U: Serialize>(
        &self,
        realm: &str,
        request: &HookRequest,
        user: &U,
    ) -> Result<()> {
        for hooks in self.apps_in_realm(realm) {
            hooks
                .dispatch(
                    USER_HOOK_COLLECTION,
                    HookEvent::UserAfterLogin,
                    request,
                    user,
                )
                .await?;
        }
        Ok(())
    }

    /// Observer fan-out for fresh signups (any track).
    pub async fn dispatch_user_after_register<U: Serialize>(
        &self,
        realm: &str,
        request: &HookRequest,
        user: &U,
    ) -> Result<()> {
        for hooks in self.apps_in_realm(realm) {
            hooks
                .dispatch(
                    USER_HOOK_COLLECTION,
                    HookEvent::UserAfterRegister,
                    request,
                    user,
                )
                .await?;
        }
        Ok(())
    }

    /// Dispatch a custom-route invocation to the `(realm, app)` hook
    /// engine. Returns `Ok(None)` when no hooks are loaded for that
    /// app OR when the app has no handler for `(method, path)`. The
    /// API layer maps both cases to HTTP 404.
    pub async fn invoke_custom_route(
        &self,
        realm: &str,
        app: &str,
        method: &str,
        path: &str,
        ctx: &CustomRouteContext,
    ) -> Result<Option<CustomRouteResponse>> {
        let Some(hooks) = self.get(realm, app) else {
            return Ok(None);
        };
        hooks.invoke_custom_route(method, path, ctx).await
    }
}

#[cfg(test)]
// Test names mirror the JS API surface (records.findOne, records.findByFilter)
// to make failures grep-able from a hook author's perspective.
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[tokio::test]
    async fn bootstrap_installs_app_global() {
        let hooks = AppHooks::new().await.unwrap();
        hooks
            .eval("$app.log('hello from boot');", "<smoke>")
            .await
            .unwrap();
        let logs = hooks.drain_logs().await.unwrap();
        assert_eq!(logs, vec!["hello from boot".to_string()]);
    }

    #[tokio::test]
    async fn after_create_handler_runs_with_payload() {
        let hooks = AppHooks::new().await.unwrap();
        hooks
            .eval(
                r#"
                $app.onRecordAfterCreate("notes", (rec) => {
                    $app.log("created:" + rec.id);
                });
                "#,
                "<test>",
            )
            .await
            .unwrap();
        hooks
            .dispatch(
                "notes",
                HookEvent::AfterCreate,
                &HookRequest::default(),
                &json!({"id":"r1"}),
            )
            .await
            .unwrap();
        let logs = hooks.drain_logs().await.unwrap();
        assert_eq!(logs, vec!["created:r1".to_string()]);
    }

    #[tokio::test]
    async fn handler_for_other_collection_is_silent() {
        let hooks = AppHooks::new().await.unwrap();
        hooks
            .eval(
                r#"$app.onRecordAfterCreate("posts", () => $app.log("posts"));"#,
                "<t>",
            )
            .await
            .unwrap();
        hooks
            .dispatch(
                "notes",
                HookEvent::AfterCreate,
                &HookRequest::default(),
                &json!({"id":"r1"}),
            )
            .await
            .unwrap();
        let logs = hooks.drain_logs().await.unwrap();
        assert!(logs.is_empty());
    }

    #[tokio::test]
    async fn multiple_handlers_for_same_event_all_fire() {
        let hooks = AppHooks::new().await.unwrap();
        hooks
            .eval(
                r#"
                $app.onRecordAfterCreate("notes", () => $app.log("a"));
                $app.onRecordAfterCreate("notes", () => $app.log("b"));
                "#,
                "<t>",
            )
            .await
            .unwrap();
        hooks
            .dispatch(
                "notes",
                HookEvent::AfterCreate,
                &HookRequest::default(),
                &json!({"id":"r1"}),
            )
            .await
            .unwrap();
        let logs = hooks.drain_logs().await.unwrap();
        assert_eq!(logs, vec!["a".to_string(), "b".to_string()]);
    }

    #[tokio::test]
    async fn handler_throws_does_not_break_dispatch() {
        let hooks = AppHooks::new().await.unwrap();
        hooks
            .eval(
                r#"
                $app.onRecordAfterCreate("notes", () => { throw new Error("boom"); });
                $app.onRecordAfterCreate("notes", () => $app.log("survived"));
                "#,
                "<t>",
            )
            .await
            .unwrap();
        hooks
            .dispatch(
                "notes",
                HookEvent::AfterCreate,
                &HookRequest::default(),
                &json!({"id":"r1"}),
            )
            .await
            .unwrap();
        let logs = hooks.drain_logs().await.unwrap();
        assert_eq!(logs, vec!["survived".to_string()]);
        let errs = hooks.drain_errors().await.unwrap();
        assert!(errs.iter().any(|e| e.contains("boom")));
    }

    // ------------- user-lifecycle hooks -------------

    #[tokio::test]
    async fn user_after_register_fires_across_every_app_in_realm() {
        let engine = HookEngine::new();
        // Two apps in the same realm; each registers an
        // onUserAfterRegister that logs the user id.
        for app in ["mobile", "web"] {
            let dir = tempdir().unwrap();
            std::fs::write(
                dir.path().join("hook.js"),
                format!(r#"$app.onUserAfterRegister((u) => $app.log("{app} saw " + u.id));"#),
            )
            .unwrap();
            engine
                .load_app("acme", app, dir.path(), None, None)
                .await
                .unwrap();
        }
        // Also load an unrelated realm — must NOT fire.
        let other = tempdir().unwrap();
        std::fs::write(
            other.path().join("hook.js"),
            r#"$app.onUserAfterRegister(() => $app.log("widgets-realm"));"#,
        )
        .unwrap();
        engine
            .load_app("widgets", "mobile", other.path(), None, None)
            .await
            .unwrap();

        engine
            .dispatch_user_after_register(
                "acme",
                &HookRequest::default(),
                &json!({"id":"u-1","email":"u@x","verified":true}),
            )
            .await
            .unwrap();

        let mut acme_mobile = engine
            .get("acme", "mobile")
            .unwrap()
            .drain_logs()
            .await
            .unwrap();
        let mut acme_web = engine
            .get("acme", "web")
            .unwrap()
            .drain_logs()
            .await
            .unwrap();
        let widgets = engine
            .get("widgets", "mobile")
            .unwrap()
            .drain_logs()
            .await
            .unwrap();
        acme_mobile.sort();
        acme_web.sort();
        assert_eq!(acme_mobile, vec!["mobile saw u-1".to_string()]);
        assert_eq!(acme_web, vec!["web saw u-1".to_string()]);
        assert!(widgets.is_empty(), "unrelated realm must not fire");
    }

    #[tokio::test]
    async fn user_before_login_veto_from_any_app_aborts() {
        let engine = HookEngine::new();
        // app A is silent, app B vetoes any login.
        let a = tempdir().unwrap();
        std::fs::write(
            a.path().join("hook.js"),
            r#"$app.onUserBeforeLogin(() => $app.log("a saw"));"#,
        )
        .unwrap();
        engine
            .load_app("acme", "a", a.path(), None, None)
            .await
            .unwrap();

        let b = tempdir().unwrap();
        std::fs::write(
            b.path().join("hook.js"),
            r#"$app.onUserBeforeLogin((u) => { throw new Error("banned: " + u.email); });"#,
        )
        .unwrap();
        engine
            .load_app("acme", "b", b.path(), None, None)
            .await
            .unwrap();

        let res = engine
            .dispatch_user_before_login(
                "acme",
                &HookRequest::default(),
                &json!({"id":"u-1","email":"banned@x","verified":true}),
            )
            .await;
        match res {
            Err(RuntimeError::Veto(msg)) => assert!(msg.contains("banned@x"), "got: {msg}"),
            other => panic!("expected Veto, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn user_after_login_is_an_observer_handler_errors_dont_propagate() {
        let engine = HookEngine::new();
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("hook.js"),
            r#"$app.onUserAfterLogin(() => { throw new Error("oops"); });"#,
        )
        .unwrap();
        engine
            .load_app("acme", "mobile", dir.path(), None, None)
            .await
            .unwrap();
        // Must NOT bubble — observer dispatch swallows handler throws.
        engine
            .dispatch_user_after_login(
                "acme",
                &HookRequest::default(),
                &json!({"id":"u-1","email":"a@x","verified":true}),
            )
            .await
            .unwrap();
        // The error landed in __rb_errors for observability.
        let errs = engine
            .get("acme", "mobile")
            .unwrap()
            .drain_errors()
            .await
            .unwrap();
        assert!(errs.iter().any(|e| e.contains("oops")), "got: {errs:?}");
    }

    #[tokio::test]
    async fn engine_load_app_picks_up_js_files() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("logger.js"),
            r#"$app.onRecordAfterCreate("notes", (r) => $app.log("hello " + r.id));"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("ignored.txt"), "not js").unwrap();
        let engine = HookEngine::new();
        let n = engine
            .load_app("acme", "mobile", dir.path(), None, None)
            .await
            .unwrap();
        assert_eq!(n, 1);

        engine
            .dispatch(
                "acme",
                "mobile",
                "notes",
                HookEvent::AfterCreate,
                &HookRequest::default(),
                &json!({"id":"x"}),
            )
            .await
            .unwrap();
        let hooks = engine.get("acme", "mobile").unwrap();
        assert_eq!(
            hooks.drain_logs().await.unwrap(),
            vec!["hello x".to_string()]
        );
    }

    #[tokio::test]
    async fn dispatch_without_loaded_hooks_is_noop() {
        let engine = HookEngine::new();
        engine
            .dispatch(
                "acme",
                "mobile",
                "notes",
                HookEvent::AfterCreate,
                &HookRequest::default(),
                &json!({}),
            )
            .await
            .unwrap();
    }

    // ------------- sandbox limits -------------

    #[tokio::test(flavor = "multi_thread")]
    async fn cpu_deadline_aborts_infinite_loop_in_dispatch() {
        let limits = SandboxLimits {
            memory_bytes: None,
            stack_bytes: None,
            cpu_time_ms: Some(50),
        };
        let hooks = AppHooks::with_records_and_limits(None, limits)
            .await
            .unwrap();
        hooks
            .eval(
                r#"$app.onRecordAfterCreate("notes", () => { while (true) {} });"#,
                "<infinite>",
            )
            .await
            .unwrap();

        let started = std::time::Instant::now();
        let res = hooks
            .dispatch(
                "notes",
                HookEvent::AfterCreate,
                &HookRequest::default(),
                &json!({"id": "n1"}),
            )
            .await;
        let elapsed = started.elapsed();

        match res {
            Err(RuntimeError::Timeout(ms)) => assert_eq!(ms, 50),
            other => panic!("expected Timeout(50), got: {other:?}"),
        }
        // 50 ms deadline; allow generous slack for CI but reject runaway.
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "took {elapsed:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cpu_deadline_disarms_so_next_dispatch_runs_clean() {
        // First call exhausts its budget; second call (with the same
        // AppHooks) must not inherit a stale deadline.
        let limits = SandboxLimits {
            memory_bytes: None,
            stack_bytes: None,
            cpu_time_ms: Some(50),
        };
        let hooks = AppHooks::with_records_and_limits(None, limits)
            .await
            .unwrap();
        hooks
            .eval(
                r#"
                let armed = false;
                $app.onRecordAfterCreate("notes", () => {
                    if (!armed) { armed = true; while (true) {} }
                    $app.log("ok");
                });
                "#,
                "<one-shot-loop>",
            )
            .await
            .unwrap();

        let first = hooks
            .dispatch(
                "notes",
                HookEvent::AfterCreate,
                &HookRequest::default(),
                &json!({}),
            )
            .await;
        assert!(
            matches!(first, Err(RuntimeError::Timeout(_))),
            "got: {first:?}"
        );

        // Second dispatch — fresh deadline, fast handler, should succeed.
        hooks
            .dispatch(
                "notes",
                HookEvent::AfterCreate,
                &HookRequest::default(),
                &json!({}),
            )
            .await
            .unwrap();
        assert_eq!(hooks.drain_logs().await.unwrap(), vec!["ok".to_string()]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn memory_limit_aborts_huge_allocation() {
        let limits = SandboxLimits {
            // 1 MiB cap is well below the 8 MiB string the hook tries to build.
            memory_bytes: Some(1024 * 1024),
            stack_bytes: None,
            cpu_time_ms: Some(2_000),
        };
        let hooks = AppHooks::with_records_and_limits(None, limits)
            .await
            .unwrap();
        hooks
            .eval(
                r#"
                $app.onRecordAfterCreate("notes", () => {
                    // Force a large allocation; the runtime must reject it.
                    let s = "x";
                    for (let i = 0; i < 23; i++) s = s + s; // ~8 MiB
                    $app.log("unexpected: " + s.length);
                });
                "#,
                "<bigalloc>",
            )
            .await
            .unwrap();

        // The handler error gets caught by the dispatch driver and
        // stashed via __rb_record_error, so dispatch returns Ok; the
        // assertion is that the unreachable log line never ran.
        hooks
            .dispatch(
                "notes",
                HookEvent::AfterCreate,
                &HookRequest::default(),
                &json!({}),
            )
            .await
            .unwrap();
        assert!(hooks.drain_logs().await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unlimited_sandbox_lets_long_hook_run() {
        // Sanity check: opting out of limits really opts out.
        let hooks = AppHooks::with_records_and_limits(None, SandboxLimits::unlimited())
            .await
            .unwrap();
        hooks
            .eval(
                r#"
                $app.onRecordAfterCreate("notes", () => {
                    let n = 0;
                    for (let i = 0; i < 100000; i++) n += i;
                    $app.log("sum:" + n);
                });
                "#,
                "<work>",
            )
            .await
            .unwrap();
        hooks
            .dispatch(
                "notes",
                HookEvent::AfterCreate,
                &HookRequest::default(),
                &json!({}),
            )
            .await
            .unwrap();
        assert_eq!(
            hooks.drain_logs().await.unwrap(),
            vec!["sum:4999950000".to_string()]
        );
    }

    #[tokio::test]
    async fn engine_load_app_strips_ts_and_runs_alongside_js() {
        let dir = tempdir().unwrap();
        // A real TS file with annotations, interface, `as`-cast: should strip + load.
        std::fs::write(
            dir.path().join("typed.ts"),
            r#"
            interface Note { id: string }
            $app.onRecordAfterCreate("notes", (r: Note): void => {
                const id = (r as any).id as string;
                $app.log("typed:" + id);
            });
            "#,
        )
        .unwrap();
        // A plain JS file in the same directory should keep working.
        std::fs::write(
            dir.path().join("plain.js"),
            r#"$app.onRecordAfterCreate("notes", (r) => $app.log("plain:" + r.id));"#,
        )
        .unwrap();
        // A syntactically broken TS file must be skipped without poisoning siblings.
        std::fs::write(dir.path().join("broken.ts"), "function (: { ").unwrap();
        // Non-script files stay ignored.
        std::fs::write(dir.path().join("README.md"), "# notes").unwrap();

        let engine = HookEngine::new();
        let n = engine
            .load_app("acme", "mobile", dir.path(), None, None)
            .await
            .unwrap();
        assert_eq!(
            n, 2,
            "expected typed.ts + plain.js to load (broken.ts skipped)"
        );

        engine
            .dispatch(
                "acme",
                "mobile",
                "notes",
                HookEvent::AfterCreate,
                &HookRequest::default(),
                &json!({"id": "abc"}),
            )
            .await
            .unwrap();

        let mut logs = engine
            .get("acme", "mobile")
            .unwrap()
            .drain_logs()
            .await
            .unwrap();
        logs.sort();
        assert_eq!(logs, vec!["plain:abc".to_string(), "typed:abc".to_string()]);
    }

    #[tokio::test]
    async fn invalid_js_file_logs_and_doesnt_poison_others() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("bad.js"), "this is not (valid;").unwrap();
        std::fs::write(
            dir.path().join("good.js"),
            r#"$app.onRecordAfterCreate("c", () => $app.log("ok"));"#,
        )
        .unwrap();
        let engine = HookEngine::new();
        let n = engine
            .load_app("acme", "mobile", dir.path(), None, None)
            .await
            .unwrap();
        assert_eq!(n, 1);

        engine
            .dispatch(
                "acme",
                "mobile",
                "c",
                HookEvent::AfterCreate,
                &HookRequest::default(),
                &json!({}),
            )
            .await
            .unwrap();
        assert_eq!(
            engine
                .get("acme", "mobile")
                .unwrap()
                .drain_logs()
                .await
                .unwrap(),
            vec!["ok".to_string()]
        );
    }

    // ------------- $app.records.* via mock bridge -------------

    /// Test double: in-memory rows keyed by (collection, id). Captures
    /// every call for assertions.
    #[derive(Default)]
    struct MockBridge {
        rows: parking_lot::Mutex<BTreeMap<(String, String), Json>>,
        creates: parking_lot::Mutex<Vec<(String, BTreeMap<String, Json>)>>,
    }

    impl MockBridge {
        fn new() -> Arc<Self> {
            Arc::new(Self::default())
        }
        fn with_row(self: &Arc<Self>, coll: &str, id: &str, row: Json) {
            self.rows
                .lock()
                .insert((coll.to_string(), id.to_string()), row);
        }
    }

    impl RecordsBridge for MockBridge {
        fn find_one(&self, collection: &str, id: &str) -> Result<Option<Json>> {
            Ok(self
                .rows
                .lock()
                .get(&(collection.to_string(), id.to_string()))
                .cloned())
        }
        fn find_by_filter(
            &self,
            collection: &str,
            _filter: &str,
            _per_page: u32,
        ) -> Result<Vec<Json>> {
            // Test stub: return every row for the collection.
            Ok(self
                .rows
                .lock()
                .iter()
                .filter(|((c, _), _)| c == collection)
                .map(|(_, v)| v.clone())
                .collect())
        }
        fn create(&self, collection: &str, fields: BTreeMap<String, Json>) -> Result<Json> {
            self.creates
                .lock()
                .push((collection.to_string(), fields.clone()));
            let id = format!("mock-{}", self.creates.lock().len());
            let row = serde_json::json!({
                "id": id,
                "collection": collection,
                "fields": fields,
            });
            self.rows
                .lock()
                .insert((collection.to_string(), id), row.clone());
            Ok(row)
        }
        fn update(
            &self,
            collection: &str,
            id: &str,
            patch: BTreeMap<String, Json>,
        ) -> Result<Json> {
            let key = (collection.to_string(), id.to_string());
            let mut rows = self.rows.lock();
            let Some(row) = rows.get_mut(&key) else {
                return Err(RuntimeError::Js(format!("not found: {collection}/{id}")));
            };
            // Apply patch to row.fields if it's an object.
            if let Some(fields) = row.get_mut("fields").and_then(Json::as_object_mut) {
                for (k, v) in patch {
                    fields.insert(k, v);
                }
            }
            Ok(row.clone())
        }
        fn delete(&self, collection: &str, id: &str) -> Result<()> {
            let key = (collection.to_string(), id.to_string());
            if self.rows.lock().remove(&key).is_none() {
                return Err(RuntimeError::Js(format!("not found: {collection}/{id}")));
            }
            Ok(())
        }
    }

    async fn hooks_with_mock(mock: Arc<MockBridge>) -> AppHooks {
        AppHooks::with_records(Some(mock as Arc<dyn RecordsBridge>))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn records_findOne_returns_record_as_object() {
        let mock = MockBridge::new();
        mock.with_row("users", "u1", serde_json::json!({"id":"u1","email":"a@x"}));
        let hooks = hooks_with_mock(mock).await;
        hooks
            .eval(
                r#"
                const u = $app.records.findOne("users", "u1");
                $app.log("email=" + u.email);
                "#,
                "<t>",
            )
            .await
            .unwrap();
        assert_eq!(
            hooks.drain_logs().await.unwrap(),
            vec!["email=a@x".to_string()]
        );
    }

    #[tokio::test]
    async fn records_findOne_missing_returns_null() {
        let hooks = hooks_with_mock(MockBridge::new()).await;
        hooks
            .eval(
                r#"
                const u = $app.records.findOne("users", "nope");
                $app.log(u === null ? "missing" : "found");
                "#,
                "<t>",
            )
            .await
            .unwrap();
        assert_eq!(
            hooks.drain_logs().await.unwrap(),
            vec!["missing".to_string()]
        );
    }

    #[tokio::test]
    async fn records_findByFilter_returns_array() {
        let mock = MockBridge::new();
        mock.with_row("notes", "n1", serde_json::json!({"id":"n1"}));
        mock.with_row("notes", "n2", serde_json::json!({"id":"n2"}));
        mock.with_row("posts", "p1", serde_json::json!({"id":"p1"}));
        let hooks = hooks_with_mock(mock).await;
        hooks
            .eval(
                r#"
                const xs = $app.records.findByFilter("notes", "1=1", 10);
                $app.log("count=" + xs.length);
                "#,
                "<t>",
            )
            .await
            .unwrap();
        assert_eq!(
            hooks.drain_logs().await.unwrap(),
            vec!["count=2".to_string()]
        );
    }

    #[tokio::test]
    async fn records_create_persists_via_bridge() {
        let mock = MockBridge::new();
        let hooks = hooks_with_mock(mock.clone()).await;
        hooks
            .eval(
                r#"
                const r = $app.records.create("audit", {action: "x", actor: "u1"});
                $app.log("created " + r.id);
                "#,
                "<t>",
            )
            .await
            .unwrap();
        let logs = hooks.drain_logs().await.unwrap();
        assert_eq!(logs, vec!["created mock-1".to_string()]);

        let creates = mock.creates.lock();
        assert_eq!(creates.len(), 1);
        assert_eq!(creates[0].0, "audit");
        assert_eq!(
            creates[0].1.get("action").and_then(|v| v.as_str()),
            Some("x")
        );
    }

    // ------------- $app.mailer.send via mock mailer -------------

    /// Minimal in-memory `Mailer` for runtime tests. Captures every
    /// send in a shared vec; never errors.
    struct MockMailer {
        sent: parking_lot::Mutex<Vec<EmailMessage>>,
    }
    impl MockMailer {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                sent: parking_lot::Mutex::new(Vec::new()),
            })
        }
        fn drain(&self) -> Vec<EmailMessage> {
            std::mem::take(&mut *self.sent.lock())
        }
    }
    #[async_trait]
    impl Mailer for MockMailer {
        async fn send(
            &self,
            msg: EmailMessage,
        ) -> std::result::Result<(), rustbase_core::MailerError> {
            self.sent.lock().push(msg);
            Ok(())
        }
    }

    /// A `Mailer` that always returns Rejected — used to prove errors
    /// propagate from JS as thrown exceptions, not silent successes.
    struct RejectingMailer;
    #[async_trait]
    impl Mailer for RejectingMailer {
        async fn send(
            &self,
            _msg: EmailMessage,
        ) -> std::result::Result<(), rustbase_core::MailerError> {
            Err(rustbase_core::MailerError::Rejected("smtp said no".into()))
        }
    }

    async fn hooks_with_mailer(mailer: Arc<dyn Mailer>) -> AppHooks {
        AppHooks::with_records_mailer_and_limits(None, Some(mailer), SandboxLimits::default())
            .await
            .unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn mailer_send_routes_through_to_bridge() {
        let mock = MockMailer::new();
        let hooks = hooks_with_mailer(mock.clone() as Arc<dyn Mailer>).await;
        hooks
            .eval(
                r#"
                $app.mailer.send({
                    from: "no-reply@app.test",
                    to: "ada@example.com",
                    subject: "hi",
                    text: "body line",
                });
                $app.log("sent");
                "#,
                "<t>",
            )
            .await
            .unwrap();
        assert_eq!(hooks.drain_logs().await.unwrap(), vec!["sent".to_string()]);
        let sent = mock.drain();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].from, "no-reply@app.test");
        assert_eq!(sent[0].to, "ada@example.com");
        assert_eq!(sent[0].subject, "hi");
        assert_eq!(sent[0].text, "body line");
        assert!(sent[0].html.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn mailer_send_preserves_optional_html() {
        let mock = MockMailer::new();
        let hooks = hooks_with_mailer(mock.clone() as Arc<dyn Mailer>).await;
        hooks
            .eval(
                r#"
                $app.mailer.send({
                    from: "a@x", to: "b@y", subject: "s",
                    text: "plain", html: "<p>fancy</p>",
                });
                "#,
                "<t>",
            )
            .await
            .unwrap();
        let sent = mock.drain();
        assert_eq!(sent[0].html.as_deref(), Some("<p>fancy</p>"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn mailer_send_throws_in_js_when_transport_rejects() {
        let hooks = hooks_with_mailer(Arc::new(RejectingMailer) as Arc<dyn Mailer>).await;
        hooks
            .eval(
                r#"
                try {
                    $app.mailer.send({from:"a", to:"b", subject:"s", text:"t"});
                    $app.log("unexpected");
                } catch (e) {
                    $app.log("caught: " + e.message);
                }
                "#,
                "<t>",
            )
            .await
            .unwrap();
        let logs = hooks.drain_logs().await.unwrap();
        assert_eq!(logs.len(), 1);
        assert!(logs[0].starts_with("caught:"), "got: {logs:?}");
        assert!(logs[0].contains("smtp said no"), "got: {logs:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn mailer_send_rejects_non_object_arg() {
        let mock = MockMailer::new();
        let hooks = hooks_with_mailer(mock.clone() as Arc<dyn Mailer>).await;
        hooks
            .eval(
                r#"
                try {
                    $app.mailer.send("not an object");
                    $app.log("unexpected");
                } catch (e) {
                    $app.log("caught: " + e.message);
                }
                "#,
                "<t>",
            )
            .await
            .unwrap();
        assert!(mock.drain().is_empty(), "transport must not see the call");
        let logs = hooks.drain_logs().await.unwrap();
        assert!(logs[0].starts_with("caught:"), "got: {logs:?}");
    }

    // ------------- mailer-lifecycle hooks (per-app) -------------

    #[tokio::test(flavor = "multi_thread")]
    async fn mailer_before_send_fires_before_transport() {
        let mock = MockMailer::new();
        let hooks = hooks_with_mailer(mock.clone() as Arc<dyn Mailer>).await;
        hooks
            .eval(
                r#"
                $app.onMailerBeforeSend((m) => $app.log("pre:" + m.to));
                $app.onMailerAfterSend((m)  => $app.log("post:" + m.to));
                $app.mailer.send({
                    from: "a@x", to: "b@y", subject: "s", text: "body",
                });
                "#,
                "<t>",
            )
            .await
            .unwrap();

        // Order matters: pre runs before transport, post runs after.
        // (mock.drain captures sends in-order; we just verify the log.)
        let logs = hooks.drain_logs().await.unwrap();
        assert_eq!(logs, vec!["pre:b@y".to_string(), "post:b@y".to_string()]);
        assert_eq!(mock.drain().len(), 1, "transport sees the send");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn mailer_before_send_throw_vetoes_send() {
        let mock = MockMailer::new();
        let hooks = hooks_with_mailer(mock.clone() as Arc<dyn Mailer>).await;
        hooks
            .eval(
                r#"
                $app.onMailerBeforeSend((m) => {
                    if (m.to.endsWith("@blocked.test")) {
                        throw new Error("recipient blocked");
                    }
                });
                try {
                    $app.mailer.send({from:"a", to:"alice@blocked.test", subject:"s", text:"t"});
                    $app.log("unexpected: send returned");
                } catch (e) {
                    $app.log("vetoed: " + e.message);
                }
                "#,
                "<t>",
            )
            .await
            .unwrap();

        let logs = hooks.drain_logs().await.unwrap();
        assert_eq!(logs, vec!["vetoed: recipient blocked".to_string()]);
        assert!(
            mock.drain().is_empty(),
            "vetoed send must not reach transport"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn mailer_after_send_observer_errors_dont_propagate() {
        // After-send is post-delivery; a throw shouldn't roll back
        // the send. It lands in __rb_errors for observability.
        let mock = MockMailer::new();
        let hooks = hooks_with_mailer(mock.clone() as Arc<dyn Mailer>).await;
        hooks
            .eval(
                r#"
                $app.onMailerAfterSend(() => { throw new Error("audit logger died"); });
                $app.mailer.send({from:"a", to:"b", subject:"s", text:"t"});
                $app.log("send returned cleanly");
                "#,
                "<t>",
            )
            .await
            .unwrap();

        let logs = hooks.drain_logs().await.unwrap();
        assert_eq!(logs, vec!["send returned cleanly".to_string()]);
        assert_eq!(mock.drain().len(), 1, "transport still got the send");
        let errs = hooks.drain_errors().await.unwrap();
        assert!(
            errs.iter().any(|e| e.contains("audit logger died")),
            "got: {errs:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn mailer_before_send_runs_in_order_first_throw_wins() {
        let mock = MockMailer::new();
        let hooks = hooks_with_mailer(mock.clone() as Arc<dyn Mailer>).await;
        hooks
            .eval(
                r#"
                $app.onMailerBeforeSend(() => $app.log("first"));
                $app.onMailerBeforeSend(() => { throw new Error("second blocks"); });
                $app.onMailerBeforeSend(() => $app.log("third (should not run)"));
                try { $app.mailer.send({from:"a", to:"b", subject:"s", text:"t"}); }
                catch (e) { $app.log("caught:" + e.message); }
                "#,
                "<t>",
            )
            .await
            .unwrap();

        let logs = hooks.drain_logs().await.unwrap();
        assert_eq!(
            logs,
            vec!["first".to_string(), "caught:second blocks".to_string()]
        );
        assert!(mock.drain().is_empty());
    }

    // ------------- $app.routerAdd custom routes -------------

    fn empty_ctx(method: &str, path: &str) -> CustomRouteContext {
        CustomRouteContext {
            method: method.into(),
            path: path.into(),
            query: BTreeMap::new(),
            headers: BTreeMap::new(),
            body: serde_json::Value::Null,
        }
    }

    #[tokio::test]
    async fn router_add_unregistered_path_returns_none() {
        let hooks = AppHooks::new().await.unwrap();
        let r = hooks
            .invoke_custom_route("GET", "/missing", &empty_ctx("GET", "/missing"))
            .await
            .unwrap();
        assert!(r.is_none());
    }

    #[tokio::test]
    async fn router_add_returns_handler_result_as_response() {
        let hooks = AppHooks::new().await.unwrap();
        hooks
            .eval(
                r#"
                $app.routerAdd("GET", "/hello", (ctx) => {
                    return { status: 200, body: { method: ctx.method, who: ctx.query.who } };
                });
                "#,
                "<t>",
            )
            .await
            .unwrap();
        let mut ctx = empty_ctx("GET", "/hello");
        ctx.query.insert("who".into(), "ada".into());
        let r = hooks
            .invoke_custom_route("GET", "/hello", &ctx)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.body["method"], "GET");
        assert_eq!(r.body["who"], "ada");
    }

    #[tokio::test]
    async fn router_add_undefined_return_becomes_204() {
        let hooks = AppHooks::new().await.unwrap();
        hooks
            .eval(r#"$app.routerAdd("DELETE", "/sink", (_ctx) => {});"#, "<t>")
            .await
            .unwrap();
        let r = hooks
            .invoke_custom_route("DELETE", "/sink", &empty_ctx("DELETE", "/sink"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(r.status, 204);
        // serde_json::Value::default() is Null, which matches the JS
        // "no body" semantics; just confirm we didn't manufacture one.
        assert!(r.body.is_null());
    }

    #[tokio::test]
    async fn router_add_non_object_return_wraps_as_200_body() {
        let hooks = AppHooks::new().await.unwrap();
        hooks
            .eval(r#"$app.routerAdd("GET", "/n", () => 42);"#, "<t>")
            .await
            .unwrap();
        let r = hooks
            .invoke_custom_route("GET", "/n", &empty_ctx("GET", "/n"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.body, serde_json::json!(42));
    }

    #[tokio::test]
    async fn router_add_handler_throw_becomes_500_and_records_error() {
        let hooks = AppHooks::new().await.unwrap();
        hooks
            .eval(
                r#"$app.routerAdd("GET", "/boom", () => { throw new Error("kapow"); });"#,
                "<t>",
            )
            .await
            .unwrap();
        let r = hooks
            .invoke_custom_route("GET", "/boom", &empty_ctx("GET", "/boom"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(r.status, 500);
        assert_eq!(r.body["error"], "kapow");
        let errs = hooks.drain_errors().await.unwrap();
        assert!(errs.iter().any(|e| e.contains("kapow")), "got: {errs:?}");
    }

    #[tokio::test]
    async fn router_add_method_match_is_case_insensitive() {
        let hooks = AppHooks::new().await.unwrap();
        hooks
            .eval(
                r#"$app.routerAdd("get", "/foo", () => ({ status: 200, body: "ok" }));"#,
                "<t>",
            )
            .await
            .unwrap();
        // Registered as "get"; uppercased internally — querying "GET"
        // should match. Querying "POST" should miss.
        assert!(
            hooks
                .invoke_custom_route("GET", "/foo", &empty_ctx("GET", "/foo"))
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            hooks
                .invoke_custom_route("POST", "/foo", &empty_ctx("POST", "/foo"))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn router_add_validates_inputs() {
        let hooks = AppHooks::new().await.unwrap();
        // Path without leading slash → throws at registration.
        let res = hooks
            .eval(
                r#"
                try { $app.routerAdd("GET", "no-slash", () => null); $app.log("unexpected"); }
                catch (e) { $app.log("rejected: " + e.message); }
                "#,
                "<t>",
            )
            .await;
        assert!(res.is_ok());
        let logs = hooks.drain_logs().await.unwrap();
        assert!(logs[0].starts_with("rejected:"), "got: {logs:?}");
    }

    #[tokio::test]
    async fn router_add_reregister_replaces_handler() {
        let hooks = AppHooks::new().await.unwrap();
        hooks
            .eval(
                r#"
                $app.routerAdd("GET", "/x", () => ({ body: "first" }));
                $app.routerAdd("GET", "/x", () => ({ body: "second" }));
                "#,
                "<t>",
            )
            .await
            .unwrap();
        let r = hooks
            .invoke_custom_route("GET", "/x", &empty_ctx("GET", "/x"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(r.body, serde_json::json!("second"));
    }

    // ------------- $app.cron scheduled jobs -------------

    #[tokio::test]
    async fn cron_registration_records_pending_job() {
        let hooks = AppHooks::new().await.unwrap();
        hooks
            .eval(
                r#"
                const id = $app.cron("0 0 * * * *", () => $app.log("hourly"));
                $app.log("id=" + id);
                "#,
                "<t>",
            )
            .await
            .unwrap();
        let logs = hooks.drain_logs().await.unwrap();
        // First registration -> id=1.
        assert_eq!(logs, vec!["id=1".to_string()]);
    }

    #[tokio::test]
    async fn cron_invoke_dispatches_to_correct_handler() {
        let hooks = AppHooks::new().await.unwrap();
        hooks
            .eval(
                r#"
                $app.cron("0 0 * * * *", () => $app.log("first"));
                $app.cron("0 0 * * * *", () => $app.log("second"));
                "#,
                "<t>",
            )
            .await
            .unwrap();
        // Drain the registration logs (none — handlers only fire on
        // invoke), then invoke each by id.
        let _ = hooks.drain_logs().await.unwrap();
        hooks.invoke_cron(1).await.unwrap();
        hooks.invoke_cron(2).await.unwrap();
        // Out-of-range id is a silent no-op (no logs added).
        hooks.invoke_cron(999).await.unwrap();
        let logs = hooks.drain_logs().await.unwrap();
        assert_eq!(logs, vec!["first".to_string(), "second".to_string()]);
    }

    #[tokio::test]
    async fn cron_handler_throw_is_caught_and_recorded() {
        let hooks = AppHooks::new().await.unwrap();
        hooks
            .eval(
                r#"$app.cron("0 0 * * * *", () => { throw new Error("kapow"); });"#,
                "<t>",
            )
            .await
            .unwrap();
        // invoke_cron returns Ok even though the handler threw —
        // dispatch is observer-style, errors land in __rb_errors.
        hooks.invoke_cron(1).await.unwrap();
        let errs = hooks.drain_errors().await.unwrap();
        assert!(errs.iter().any(|e| e.contains("kapow")), "got: {errs:?}");
    }

    #[tokio::test]
    async fn cron_validates_inputs_at_registration() {
        let hooks = AppHooks::new().await.unwrap();
        hooks
            .eval(
                r#"
                try { $app.cron(42, () => null); $app.log("unexpected: number"); }
                catch (e) { $app.log("rejected: " + e.message); }
                try { $app.cron("0 0 * * * *", null); $app.log("unexpected: null fn"); }
                catch (e) { $app.log("rejected: " + e.message); }
                "#,
                "<t>",
            )
            .await
            .unwrap();
        let logs = hooks.drain_logs().await.unwrap();
        assert_eq!(logs.len(), 2);
        assert!(
            logs.iter().all(|l| l.starts_with("rejected:")),
            "got: {logs:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn start_cron_tasks_spawns_jobs_and_fires_them() {
        // Use a 1-second cron expression so the test finishes quickly.
        // (Format is `sec min hour dom mon dow`; "* * * * * *" = every second.)
        let hooks = Arc::new(AppHooks::new().await.unwrap());
        hooks
            .eval(
                r#"$app.cron("* * * * * *", () => $app.log("tick"));"#,
                "<t>",
            )
            .await
            .unwrap();
        let n = hooks.start_cron_tasks().await.unwrap();
        assert_eq!(n, 1, "exactly one task spawned");
        // Wait long enough for at least one tick.
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        let logs = hooks.drain_logs().await.unwrap();
        assert!(!logs.is_empty(), "expected at least one tick log");
        assert!(logs.iter().all(|l| l == "tick"), "got: {logs:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn start_cron_tasks_skips_invalid_expressions() {
        let hooks = Arc::new(AppHooks::new().await.unwrap());
        hooks
            .eval(
                r#"
                $app.cron("not a cron expr", () => $app.log("bad"));
                $app.cron("* * * * * *", () => $app.log("good"));
                "#,
                "<t>",
            )
            .await
            .unwrap();
        let n = hooks.start_cron_tasks().await.unwrap();
        assert_eq!(n, 1, "only the valid expression spawned");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn start_cron_tasks_is_idempotent_when_pending_is_empty() {
        let hooks = Arc::new(AppHooks::new().await.unwrap());
        // No $app.cron calls — pending is empty.
        assert_eq!(hooks.start_cron_tasks().await.unwrap(), 0);
        // Re-calling is also a no-op.
        assert_eq!(hooks.start_cron_tasks().await.unwrap(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn drop_aborts_cron_tasks() {
        // Spawn a job; drop the AppHooks while the task is sleeping.
        // The task's Weak::upgrade fails on the next wake, OR the
        // explicit abort fires first. Either way the test must not
        // hang waiting for the task to exit.
        let hooks = Arc::new(AppHooks::new().await.unwrap());
        hooks
            .eval(r#"$app.cron("* * * * * *", () => null);"#, "<t>")
            .await
            .unwrap();
        let n = hooks.start_cron_tasks().await.unwrap();
        assert_eq!(n, 1);
        // Pull a JoinHandle clone so we can assert the abort took.
        let handle_present = !hooks.cron_tasks.lock().is_empty();
        assert!(handle_present);
        drop(hooks);
        // Give the runtime a moment to observe the abort.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        // Nothing to assert structurally — we just need the test to
        // not hang. If Drop didn't abort, the runtime might keep the
        // task scheduled past the harness, which would surface as a
        // tokio panic in CI.
    }

    #[tokio::test]
    async fn mailer_unavailable_when_no_bridge_throws() {
        // AppHooks with no mailer bound: the JS shim is still present
        // (consistent surface across dev / test) but every call throws.
        let hooks = AppHooks::new().await.unwrap();
        hooks
            .eval(
                r#"
                try {
                    $app.mailer.send({from:"a", to:"b", subject:"s", text:"t"});
                    $app.log("unexpected");
                } catch (e) {
                    $app.log("caught: " + e.message);
                }
                "#,
                "<t>",
            )
            .await
            .unwrap();
        let logs = hooks.drain_logs().await.unwrap();
        assert!(logs[0].starts_with("caught:"), "got: {logs:?}");
    }

    #[tokio::test]
    async fn records_unavailable_when_no_bridge_throws() {
        let hooks = AppHooks::new().await.unwrap();
        // No bridge wired → calling $app.records.findOne should throw.
        hooks
            .eval(
                r#"
                try {
                    $app.records.findOne("x", "y");
                    $app.log("unexpected");
                } catch (e) {
                    $app.log("caught: " + e.message);
                }
                "#,
                "<t>",
            )
            .await
            .unwrap();
        let logs = hooks.drain_logs().await.unwrap();
        assert_eq!(logs.len(), 1);
        assert!(logs[0].starts_with("caught:"), "got: {logs:?}");
    }

    #[tokio::test]
    async fn records_after_create_hook_creates_audit_row() {
        // Demonstrates the canonical use case: an after_create hook
        // that writes a derived record.
        let mock = MockBridge::new();
        let hooks = hooks_with_mock(mock.clone()).await;
        hooks
            .eval(
                r#"
                $app.onRecordAfterCreate("notes", (note) => {
                    $app.records.create("audit", {
                        action: "note.created",
                        ref: note.id,
                    });
                });
                "#,
                "<t>",
            )
            .await
            .unwrap();
        hooks
            .dispatch(
                "notes",
                HookEvent::AfterCreate,
                &HookRequest::default(),
                &serde_json::json!({"id":"n42"}),
            )
            .await
            .unwrap();
        let creates = mock.creates.lock();
        assert_eq!(creates.len(), 1);
        assert_eq!(creates[0].0, "audit");
        assert_eq!(
            creates[0].1.get("ref").and_then(|v| v.as_str()),
            Some("n42")
        );
    }

    #[tokio::test]
    async fn records_update_merges_patch_into_row() {
        let mock = MockBridge::new();
        mock.with_row(
            "notes",
            "n1",
            serde_json::json!({"id":"n1","collection":"notes","fields":{"title":"old","pinned":false}}),
        );
        let hooks = hooks_with_mock(mock.clone()).await;
        hooks
            .eval(
                r#"
                const r = $app.records.update("notes", "n1", {title: "new"});
                $app.log("title=" + r.fields.title + " pinned=" + r.fields.pinned);
                "#,
                "<t>",
            )
            .await
            .unwrap();
        assert_eq!(
            hooks.drain_logs().await.unwrap(),
            vec!["title=new pinned=false".to_string()]
        );
    }

    #[tokio::test]
    async fn records_update_on_missing_throws() {
        let hooks = hooks_with_mock(MockBridge::new()).await;
        hooks
            .eval(
                r#"
                try { $app.records.update("notes", "ghost", {x: 1}); $app.log("noop"); }
                catch (e) { $app.log("threw"); }
                "#,
                "<t>",
            )
            .await
            .unwrap();
        assert_eq!(hooks.drain_logs().await.unwrap(), vec!["threw".to_string()]);
    }

    #[tokio::test]
    async fn records_delete_removes_row() {
        let mock = MockBridge::new();
        mock.with_row("notes", "n1", serde_json::json!({"id":"n1"}));
        let hooks = hooks_with_mock(mock.clone()).await;
        hooks
            .eval(
                r#"
                $app.records.delete("notes", "n1");
                const after = $app.records.findOne("notes", "n1");
                $app.log(after === null ? "gone" : "still here");
                "#,
                "<t>",
            )
            .await
            .unwrap();
        assert_eq!(hooks.drain_logs().await.unwrap(), vec!["gone".to_string()]);
    }

    #[tokio::test]
    async fn records_delete_on_missing_throws() {
        let hooks = hooks_with_mock(MockBridge::new()).await;
        hooks
            .eval(
                r#"
                try { $app.records.delete("notes", "ghost"); $app.log("noop"); }
                catch (e) { $app.log("threw"); }
                "#,
                "<t>",
            )
            .await
            .unwrap();
        assert_eq!(hooks.drain_logs().await.unwrap(), vec!["threw".to_string()]);
    }

    // ------------- before-hook tests -------------

    fn payload(pairs: &[(&str, Json)]) -> BTreeMap<String, Json> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[tokio::test]
    async fn before_create_no_hook_returns_input_unchanged() {
        let hooks = AppHooks::new().await.unwrap();
        let out = hooks
            .dispatch_before_create(
                "notes",
                &HookRequest::default(),
                payload(&[("title", serde_json::json!("x"))]),
            )
            .await
            .unwrap();
        assert_eq!(out.get("title"), Some(&serde_json::json!("x")));
    }

    #[tokio::test]
    async fn before_create_hook_can_mutate_payload() {
        let hooks = AppHooks::new().await.unwrap();
        hooks
            .eval(
                r#"
                $app.onRecordBeforeCreate("notes", (payload) => {
                    payload.title = payload.title.toUpperCase();
                    payload.processed = true;
                });
                "#,
                "<t>",
            )
            .await
            .unwrap();
        let out = hooks
            .dispatch_before_create(
                "notes",
                &HookRequest::default(),
                payload(&[("title", serde_json::json!("hello"))]),
            )
            .await
            .unwrap();
        assert_eq!(out.get("title"), Some(&serde_json::json!("HELLO")));
        assert_eq!(out.get("processed"), Some(&serde_json::json!(true)));
    }

    #[tokio::test]
    async fn before_create_hook_can_veto_with_thrown_error() {
        let hooks = AppHooks::new().await.unwrap();
        hooks
            .eval(
                r#"
                $app.onRecordBeforeCreate("notes", (payload) => {
                    if (!payload.title) throw new Error("title is required");
                });
                "#,
                "<t>",
            )
            .await
            .unwrap();
        let err = hooks
            .dispatch_before_create("notes", &HookRequest::default(), payload(&[]))
            .await
            .unwrap_err();
        assert!(matches!(err, RuntimeError::Veto(ref m) if m.contains("title is required")));
    }

    #[tokio::test]
    async fn before_create_chains_multiple_hooks_in_order() {
        let hooks = AppHooks::new().await.unwrap();
        hooks
            .eval(
                r#"
                $app.onRecordBeforeCreate("notes", (p) => { p.x = (p.x || 0) + 1; });
                $app.onRecordBeforeCreate("notes", (p) => { p.x = p.x * 10; });
                "#,
                "<t>",
            )
            .await
            .unwrap();
        let out = hooks
            .dispatch_before_create("notes", &HookRequest::default(), payload(&[]))
            .await
            .unwrap();
        assert_eq!(out.get("x"), Some(&serde_json::json!(10)));
    }

    #[tokio::test]
    async fn before_update_sees_existing_and_mutates_patch() {
        let hooks = AppHooks::new().await.unwrap();
        hooks
            .eval(
                r#"
                $app.onRecordBeforeUpdate("notes", (existing, patch) => {
                    // forbid changing the owner
                    if ("owner" in patch && patch.owner !== existing.fields.owner) {
                        throw new Error("owner is immutable");
                    }
                    patch.updated_by = "system";
                });
                "#,
                "<t>",
            )
            .await
            .unwrap();
        let existing = serde_json::json!({
            "id": "r1",
            "collection": "notes",
            "fields": { "owner": "u1", "title": "old" },
        });

        // Good patch: owner unchanged
        let out = hooks
            .dispatch_before_update(
                "notes",
                &HookRequest::default(),
                &existing,
                payload(&[("title", serde_json::json!("new"))]),
            )
            .await
            .unwrap();
        assert_eq!(out.get("title"), Some(&serde_json::json!("new")));
        assert_eq!(out.get("updated_by"), Some(&serde_json::json!("system")));

        // Bad patch: owner changed → veto
        let err = hooks
            .dispatch_before_update(
                "notes",
                &HookRequest::default(),
                &existing,
                payload(&[("owner", serde_json::json!("u2"))]),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, RuntimeError::Veto(ref m) if m.contains("owner is immutable")));
    }

    #[tokio::test]
    async fn before_delete_can_veto() {
        let hooks = AppHooks::new().await.unwrap();
        hooks
            .eval(
                r#"
                $app.onRecordBeforeDelete("notes", (existing) => {
                    if (existing.fields.locked) throw new Error("record is locked");
                });
                "#,
                "<t>",
            )
            .await
            .unwrap();

        let unlocked = serde_json::json!({"id":"r1","fields":{"locked":false}});
        hooks
            .dispatch_before_delete("notes", &HookRequest::default(), &unlocked)
            .await
            .unwrap();

        let locked = serde_json::json!({"id":"r2","fields":{"locked":true}});
        let err = hooks
            .dispatch_before_delete("notes", &HookRequest::default(), &locked)
            .await
            .unwrap_err();
        assert!(matches!(err, RuntimeError::Veto(ref m) if m.contains("locked")));
    }

    #[tokio::test]
    async fn before_create_sees_app_request_auth_id() {
        let hooks = AppHooks::new().await.unwrap();
        hooks
            .eval(
                r#"
                $app.onRecordBeforeCreate("notes", (p) => {
                    p.owner = $app.request.auth.id;
                    p.role = $app.request.auth.role;
                });
                "#,
                "<t>",
            )
            .await
            .unwrap();
        let req = HookRequest {
            auth: Some(HookAuth {
                id: "u123".into(),
                role: "user".into(),
                realm: Some("acme".into()),
            }),
            realm: "acme".into(),
            app: "mobile".into(),
            collection: "notes".into(),
        };
        let out = hooks
            .dispatch_before_create(
                "notes",
                &req,
                payload(&[("title", serde_json::json!("hi"))]),
            )
            .await
            .unwrap();
        assert_eq!(out.get("owner"), Some(&serde_json::json!("u123")));
        assert_eq!(out.get("role"), Some(&serde_json::json!("user")));
    }

    #[tokio::test]
    async fn app_request_is_cleared_after_dispatch() {
        let hooks = AppHooks::new().await.unwrap();
        hooks
            .eval(r#"$app.onRecordBeforeCreate("notes", () => {});"#, "<t>")
            .await
            .unwrap();
        let req = HookRequest::system("acme", "mobile", "notes");
        hooks
            .dispatch_before_create("notes", &req, payload(&[]))
            .await
            .unwrap();
        // After dispatch returns, $app.request must be null again so a
        // later internal eval can't see the stale principal.
        hooks
            .eval(
                "$app.log($app.request === null ? 'cleared' : 'STALE');",
                "<probe>",
            )
            .await
            .unwrap();
        assert_eq!(
            hooks.drain_logs().await.unwrap(),
            vec!["cleared".to_string()]
        );
    }

    #[tokio::test]
    async fn after_create_sees_app_request_realm_app_collection() {
        let hooks = AppHooks::new().await.unwrap();
        hooks
            .eval(
                r#"
                $app.onRecordAfterCreate("notes", (rec) => {
                    $app.log("ctx:" + $app.request.realm + "/" + $app.request.app + "/" + $app.request.collection);
                });
                "#,
                "<t>",
            )
            .await
            .unwrap();
        hooks
            .dispatch(
                "notes",
                HookEvent::AfterCreate,
                &HookRequest::system("acme", "mobile", "notes"),
                &serde_json::json!({"id":"r1"}),
            )
            .await
            .unwrap();
        assert_eq!(
            hooks.drain_logs().await.unwrap(),
            vec!["ctx:acme/mobile/notes".to_string()]
        );
    }

    #[tokio::test]
    async fn before_hook_for_other_collection_is_silent() {
        let hooks = AppHooks::new().await.unwrap();
        hooks
            .eval(
                r#"$app.onRecordBeforeCreate("posts", () => { throw new Error("not me"); });"#,
                "<t>",
            )
            .await
            .unwrap();
        // Dispatching on "notes" must not trip the posts hook.
        let out = hooks
            .dispatch_before_create(
                "notes",
                &HookRequest::default(),
                payload(&[("title", serde_json::json!("ok"))]),
            )
            .await
            .unwrap();
        assert_eq!(out.get("title"), Some(&serde_json::json!("ok")));
    }
}
