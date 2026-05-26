//! Embedded JS hook runtime for RustBase.
//!
//! One `AppHooks` per (realm, app). Each holds its own `AsyncRuntime`
//! + `AsyncContext`. At load time we evaluate every `.js` file under
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
use serde::Serialize;
use serde_json::Value as Json;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("js error: {0}")]
    Js(String),
}

pub type Result<T> = std::result::Result<T, RuntimeError>;

impl From<rquickjs::Error> for RuntimeError {
    fn from(e: rquickjs::Error) -> Self {
        RuntimeError::Js(e.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    AfterCreate,
    AfterUpdate,
    AfterDelete,
}

impl HookEvent {
    fn as_str(&self) -> &'static str {
        match self {
            HookEvent::AfterCreate => "after_create",
            HookEvent::AfterUpdate => "after_update",
            HookEvent::AfterDelete => "after_delete",
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
    fn find_by_filter(
        &self,
        collection: &str,
        filter: &str,
        per_page: u32,
    ) -> Result<Vec<Json>>;
    fn create(&self, collection: &str, fields: BTreeMap<String, Json>) -> Result<Json>;
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
    fn find_by_filter(
        &self,
        collection: &str,
        filter: &str,
        per_page: u32,
    ) -> Result<Vec<Json>> {
        let inner = self.0.clone();
        let collection = collection.to_string();
        let filter = filter.to_string();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                inner.find_by_filter(&collection, &filter, per_page).await
            })
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
}

/// JS host bound to one (realm, app). Multiple `.js` files share its
/// QuickJS context, so they can cooperate via globals.
pub struct AppHooks {
    ctx: AsyncContext,
    #[allow(dead_code)]
    rt: AsyncRuntime,
    records: Option<Arc<dyn RecordsBridge>>,
}

impl AppHooks {
    pub async fn new() -> Result<Self> {
        Self::with_records(None).await
    }

    pub async fn with_records(records: Option<Arc<dyn RecordsBridge>>) -> Result<Self> {
        let rt = AsyncRuntime::new()?;
        let ctx = AsyncContext::full(&rt).await?;
        let me = Self {
            rt,
            ctx,
            records,
        };
        me.install_app_global().await?;
        Ok(me)
    }

    /// Evaluate a single JS source. Errors are returned for visibility
    /// (the caller logs them); a failing file does not poison the
    /// context — later hooks may still load and run.
    pub async fn eval(&self, src: &str, label: &str) -> Result<()> {
        let src = src.to_string();
        let label = label.to_string();
        self.ctx
            .with(move |ctx| {
                let _: rquickjs::Value =
                    ctx.eval(src.as_bytes()).catch(&ctx).map_err(|e| {
                        RuntimeError::Js(format!("{label}: {e}"))
                    })?;
                Ok::<_, RuntimeError>(())
            })
            .await?;
        Ok(())
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
            if path.extension().and_then(|s| s.to_str()) != Some("js") {
                continue;
            }
            let label = path.display().to_string();
            let src = std::fs::read_to_string(&path)?;
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
        payload: &T,
    ) -> Result<()> {
        let key = format!("{}:{}", collection, event.as_str());
        let json = serde_json::to_string(payload)
            .map_err(|e| RuntimeError::Js(format!("serialise payload: {e}")))?;
        let driver = format!(
            r#"
            (function() {{
                const list = (globalThis.__rb_handlers || {{}})[{key}] || [];
                const payload = JSON.parse({payload_lit});
                for (const fn of list) {{
                    try {{ fn(payload); }}
                    catch (e) {{ globalThis.__rb_record_error(String(e)); }}
                }}
            }})();
            "#,
            key = json_quote(&key),
            payload_lit = json_quote(&json),
        );
        self.ctx
            .with(move |ctx| {
                let _: rquickjs::Value =
                    ctx.eval(driver.as_bytes()).catch(&ctx).map_err(|e| {
                        RuntimeError::Js(format!("dispatch: {e}"))
                    })?;
                Ok::<_, RuntimeError>(())
            })
            .await?;
        Ok(())
    }

    async fn install_app_global(&self) -> Result<()> {
        // Pure-JS half: handler registry, error collector, $app.log
        // that ALSO stashes to __rb_log so tests can drain it.
        const BOOTSTRAP_JS: &str = r#"
            (function() {
                const handlers = (globalThis.__rb_handlers = {});
                const errors = (globalThis.__rb_errors = []);
                globalThis.__rb_log = [];
                globalThis.__rb_record_error = function(msg) { errors.push(String(msg)); };

                function register(kind, collection, fn) {
                    if (typeof fn !== 'function') {
                        throw new Error('handler must be a function');
                    }
                    const key = collection + ':' + kind;
                    (handlers[key] = handlers[key] || []).push(fn);
                }

                globalThis.$app = {
                    onRecordAfterCreate(collection, fn) { register('after_create', collection, fn); },
                    onRecordAfterUpdate(collection, fn) { register('after_update', collection, fn); },
                    onRecordAfterDelete(collection, fn) { register('after_delete', collection, fn); },
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
        self.ctx
            .with(move |ctx| {
                use rquickjs::Function;

                let log_fn =
                    Function::new(ctx.clone(), |msg: String| {
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

                Ok::<_, rquickjs::Error>(())
            })
            .await?;

        // JS shim wrapping the natives in JSON-typed accessors.
        const RECORDS_SHIM: &str = r#"
            (function() {
                const ERR = '__rb_err:';
                function call(name, args) {
                    if (typeof globalThis[name] !== 'function') {
                        throw new Error("$app.records." + name + " is not available (no bridge bound)");
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
    ctx.globals().set("__rb_records_findByFilter", find_by_filter)?;

    let b3 = bridge;
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
    /// and any previously-registered handlers vanish. The optional
    /// `bridge` exposes `$app.records.*` to user code.
    pub async fn load_app(
        &self,
        realm: &str,
        app: &str,
        dir: &Path,
        bridge: Option<Arc<dyn RecordsBridge>>,
    ) -> Result<usize> {
        let hooks = AppHooks::with_records(bridge).await?;
        let loaded = hooks.load_dir(dir).await?;
        self.apps
            .insert((realm.to_string(), app.to_string()), Arc::new(hooks));
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
        payload: &T,
    ) -> Result<()> {
        let Some(hooks) = self.get(realm, app) else {
            return Ok(());
        };
        hooks.dispatch(collection, event, payload).await
    }
}

#[cfg(test)]
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
            .dispatch("notes", HookEvent::AfterCreate, &json!({"id":"r1"}))
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
            .dispatch("notes", HookEvent::AfterCreate, &json!({"id":"r1"}))
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
            .dispatch("notes", HookEvent::AfterCreate, &json!({"id":"r1"}))
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
            .dispatch("notes", HookEvent::AfterCreate, &json!({"id":"r1"}))
            .await
            .unwrap();
        let logs = hooks.drain_logs().await.unwrap();
        assert_eq!(logs, vec!["survived".to_string()]);
        let errs = hooks.drain_errors().await.unwrap();
        assert!(errs.iter().any(|e| e.contains("boom")));
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
        let n = engine.load_app("acme", "mobile", dir.path(), None).await.unwrap();
        assert_eq!(n, 1);

        engine
            .dispatch(
                "acme",
                "mobile",
                "notes",
                HookEvent::AfterCreate,
                &json!({"id":"x"}),
            )
            .await
            .unwrap();
        let hooks = engine.get("acme", "mobile").unwrap();
        assert_eq!(hooks.drain_logs().await.unwrap(), vec!["hello x".to_string()]);
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
                &json!({}),
            )
            .await
            .unwrap();
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
        let n = engine.load_app("acme", "mobile", dir.path(), None).await.unwrap();
        assert_eq!(n, 1);

        engine
            .dispatch(
                "acme",
                "mobile",
                "c",
                HookEvent::AfterCreate,
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
                .filter_map(|((c, _), v)| (c == collection).then(|| v.clone()))
                .collect())
        }
        fn create(
            &self,
            collection: &str,
            fields: BTreeMap<String, Json>,
        ) -> Result<Json> {
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
            .dispatch("notes", HookEvent::AfterCreate, &serde_json::json!({"id":"n42"}))
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
}
