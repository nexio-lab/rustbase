# RustBase

A single-binary, single-file Backend-as-a-Service in Rust. PocketBase-style:
drop one executable on a server, run the setup wizard, and you have realms,
apps, collections, auth, realtime, file storage, a dashboard, and a REST API.
SQLite under the hood for maximum operational simplicity.

---

## Mental model

RustBase organizes everything under a three-level hierarchy:

```
System
  └── Realm (identity / organization boundary; users live here)
        └── App (data product; collections + records + files live here)
```

### Master realm

On first boot, RustBase creates a single privileged realm called **the master
realm**. Its rules:

- Cannot be deleted. Its name and slug can be changed by the master admin.
- Owns the **master admin(s)** — accounts that can administer the entire server.
- Is the only place from which other realms can be created, edited, or deleted.
- A cascade-delete of a non-master realm removes every app, user, file, and
  audit record under that realm in one transaction.
- The master admin is created on first dashboard visit via a setup wizard
  (PocketBase-style).

### Realms

- Hold the user pool, OAuth provider configuration, branding, and realm-level
  policy.
- Users authenticate against a realm. A successful login produces a token
  bound to `(realm_id, user_id)`; that token can be used by any app in the
  realm. Apps still enforce per-collection access rules.
- A realm has its own **realm admin(s)**, scoped to that realm.
- Non-master realms can be created, renamed, and deleted only by master admins.

### Apps

- An app is what a developer ships against — its own collections, records,
  schema, access rules, files, and JS/TS hooks.
- An app has its own **app admin(s)** (a subset of realm admins, plus
  app-scoped admins).
- Apps inside the same realm share the realm's user pool, so SSO across apps
  is automatic.

---

## Hierarchical configuration

Every config field declares a *policy kind* that controls how it cascades. A
parent level sets bounds; a child level sets values that must fit inside those
bounds.

| Kind | Parent sets | Child can do |
|---|---|---|
| `Range<T>` | `[min, max]` + default | Pick any value inside, or tighten the range further |
| `Toggle` | `Locked(true)` / `Locked(false)` / `Open(default)` | If `Open`, choose freely; if `Locked`, cannot change |
| `EnumSet<T>` | Allowed set | Pick any subset; cannot add values |
| `Free<T>` | Default | Override freely |

Validation walks the chain at **write time** — `app value` must fit inside
`realm bound`, which must fit inside `master bound`. Reads never recompute;
stored values are already proven valid.

### Master tightens a bound after children have set values

Action: **auto-clamp and audit.** When master narrows a bound (or removes an
enum value, or flips Open → Locked), the system:

1. Locates every realm and app whose stored value falls outside the new bound.
2. Clamps each non-compliant value to the new bound (or to the new default for
   removed enum values / `Locked` toggles), in one transaction across
   `system.db`, every affected `realm.db`, and every affected app `data.db`.
3. Writes an audit entry per change to the master audit log AND the affected
   realm/app audit log, so realm admins see what happened on their next visit.

The same machinery applies to realm → app when a realm tightens its bounds.

### Who can edit what

- Master config + bounds: master admins only.
- Realm config + bounds for apps: realm admins (within master bounds) and
  master admins.
- App config: app admins (within realm bounds), realm admins, master admins.

### Candidates for hierarchical policy

- Password policy (length range, character-class toggles, history depth)
- Token TTLs (access, refresh)
- Rate limits (requests/min per IP, per user, per app)
- File storage (max upload size, allowed MIME types, per-realm quota)
- OAuth providers (which are enabled and with what scopes)
- Email/SMTP (locked sender domain, allowed templates)
- JS/TS hook capabilities (network access, fs access, CPU time, memory)
- Audit log retention
- Realtime subscription count per connection

---

## Storage layout

SQLite via `sqlx`. One file per scope, all under `data/`:

```
data/
  system.db                           # realms registry, master admins, master config + bounds, server audit log
  realms/
    <realm_id>/
      realm.db                        # users, oauth, settings, branding, realm config + bounds, refresh tokens
      storage/                        # realm-level files
      apps/
        <app_id>/
          data.db                     # collections, records, access rules, app config, app audit
          storage/                    # app-level files
  hooks/
    <realm_id>/<app_id>/              # JS/TS hook source files
```

Per-connection PRAGMAs (set on every pool):
- `journal_mode=WAL`
- `foreign_keys=ON`
- `busy_timeout=5000`
- `synchronous=NORMAL`

### Pool management

Three pool kinds, each managed independently:

| Pool | Key | Cap (default) | Notes |
|---|---|---|---|
| System pool | — | always open | one DB, opened at boot |
| Realm pool | `RealmId` | `realm_pool_cap = 32` | LRU eviction |
| App pool | `(RealmId, AppId)` | `app_pool_cap = 64` | LRU eviction |

Cold realms/apps stay on disk; their pools reopen lazily on next access in ~1 ms.

### Replication (Litestream)

Optional, off by default. Configured under `[litestream]` in `rustbase.toml`:

```toml
[litestream]
enabled = true
bucket = "s3://my-rustbase-backups"
prefix = "prod"
replicate_interval_sec = 10
```

When enabled, the server manages Litestream as a sidecar for `system.db`,
every `realm.db`, and every app `data.db`. Disabling it requires no code
changes.

---

## Crate layout

| Crate | Purpose |
|---|---|
| `rustbase-core` | IO-free domain types: `RealmId`, `AppId`, `Record`, `Schema`, `FilterNode`, `ConfigPolicy`, error enum, filter parser |
| `rustbase-db` | SQLite layer: system / realm / app pools, migrations, CRUD, filter → SQL, cascade-delete, auto-clamp engine |
| `rustbase-auth` | JWT, argon2, OAuth2, OTP, master / realm / app admin model |
| `rustbase-realtime` | In-process pub/sub broker |
| `rustbase-storage` | Local + S3 file storage via `object_store` |
| `rustbase-runtime` | Embedded JS/TS runtime, hook dispatch, sandboxing |
| `rustbase-api` | axum handlers (REST, SSE, WebSocket), `ApiError → IntoResponse` |
| `rustbase-server` | Binary: config, bootstrap, setup wizard, embedded dashboard |

Dependency rules:
- `rustbase-core` depends on no IO crate.
- Every other crate depends on `rustbase-core`.
- `rustbase-api` depends on `rustbase-db`, `rustbase-auth`, `rustbase-realtime`, `rustbase-storage`, `rustbase-runtime`.
- `rustbase-server` depends only on `rustbase-api`.

---

## Filter parser

`rustbase-core` contains a `nom`-based parser that produces a `FilterNode` AST.
The parser is IO-free and dialect-agnostic.

```rust
pub enum FilterNode {
    And(Box<FilterNode>, Box<FilterNode>),
    Or(Box<FilterNode>, Box<FilterNode>),
    Not(Box<FilterNode>),
    Eq(String, Value),
    Gt(String, Value),
    // ...
}
```

`rustbase-db` translates a `FilterNode` into a parameterized SQL `WHERE` clause
via a `filter_to_sql` function. No string interpolation of user input — every
literal becomes a bound parameter.

The same AST is reused by:
- The dashboard for client-side validation.
- The per-collection access-rules engine.
- The JS/TS hooks API for `$app.records.findRecordsByFilter(...)`.

---

## Error handling

Use `thiserror`. One error enum per crate.

```rust
// rustbase-core
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("record not found: {collection}/{id}")]
    NotFound { collection: String, id: String },
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("realm not found: {0}")]
    RealmNotFound(String),
    #[error("app not found: {realm}/{app}")]
    AppNotFound { realm: String, app: String },
    #[error("policy violation: {field} = {value} outside bound {bound}")]
    PolicyViolation { field: String, value: String, bound: String },
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("internal error: {0}")]
    Internal(String),
}
```

`rustbase-db` maps `sqlx::Error` into `CoreError` at the boundary.
`ApiError` in `rustbase-api` implements `IntoResponse` and maps `CoreError`
variants to HTTP status codes.

---

## Auth

Lives in `rustbase-auth`. Depends only on `rustbase-core`.

- **Master admins**: stored in `system.db`. Login at `/_/auth/login` with email + password.
- **Realm admins**: stored in the realm's `realm.db`. Login at `/api/realms/<realm>/auth/admin/login`.
- **App admins**: stored in the realm's `realm.db`, scoped to one or more apps.
- **End users**: stored in the realm's `realm.db`, share auth across all apps in the realm.

Token model:
- **Access tokens**: stateless JWT, 15-minute TTL (configurable via hierarchical policy), signed with HS256 or RS256 per realm. Carries `realm_id`, optional `app_id`, `user_id` or `admin_id`, `role`.
- **Refresh tokens**: opaque random strings stored in the realm's `_refresh_tokens` table, exchanged at `/auth/refresh`.
- **Revocation**: in-memory `DashSet<(RealmId, UserId)>` checked by middleware; entries auto-expire on access-token TTL.
- **Password hashing**: `argon2` (default), `bcrypt` available via feature flag.
- **OAuth2**: `oauth2` crate, providers configured per realm (allowed set bounded by master).
- **OTP**: TOTP via `totp-rs`, email OTP via the mailer.

Auth collections (`CollectionKind::Auth`) automatically include:
`email`, `password_hash`, `verified`, `last_login`, `oauth_providers`.

---

## Realtime

`rustbase-realtime` is an in-process pub/sub broker on `tokio::sync::broadcast`.

```rust
pub struct RealtimeBroker {
    channels: DashMap<SubscriptionKey, broadcast::Sender<RealtimeEvent>>,
}

pub struct SubscriptionKey {
    pub realm_id: RealmId,
    pub app_id: AppId,
    pub collection: String,
    pub record_id: Option<String>,
}
```

SSE and WebSocket handlers in `rustbase-api` subscribe to the broker.
DB hooks (post-create / post-update / post-delete) and JS/TS hooks publish to it.

---

## File storage

`rustbase-storage` uses `object_store` for backend abstraction.

Supported:
- `LocalStorage` — disk under `data/realms/<realm>/storage/` and `data/realms/<realm>/apps/<app>/storage/`.
- `S3Storage` — any S3-compatible endpoint (AWS, Cloudflare R2, MinIO).

File records store metadata in the relevant `data.db`. Binary data always
goes through the storage backend, never through the DB.

---

## Migration system

Migrations are scoped — system, realm, or app — and versioned with timestamp IDs.

```rust
pub struct Migration {
    pub id: String,           // "20240501_120000_create_users"
    pub scope: MigrationScope, // System | Realm | App
    pub up: MigrationFn,
    pub down: Option<MigrationFn>,
}

pub type MigrationFn = Box<
    dyn Fn(&SqlitePool) -> BoxFuture<Result<()>> + Send + Sync,
>;
```

Bootstrap order: system migrations → for each realm, realm migrations → for
each app, app migrations.

---

## JS/TS extensibility

`rustbase-runtime` embeds [`rquickjs`](https://github.com/DelSkayn/rquickjs)
(QuickJS bindings, no Node.js needed). Hooks live in `data/hooks/<realm>/<app>/`
as `.js` or `.ts` files (TS is transpiled at load time via `swc`).

Hook entry points (PocketBase parity):
- Record lifecycle: `onRecordBeforeCreate`, `onRecordAfterCreate`, `onRecordBeforeUpdate`, `onRecordAfterUpdate`, `onRecordBeforeDelete`, `onRecordAfterDelete`
- Auth lifecycle: `onUserBeforeLogin`, `onUserAfterLogin`, `onUserAfterRegister`
- HTTP: `routerAdd("GET", "/custom-route", handler)` for custom endpoints
- Mailer: `onMailerBeforeRecordCreateSend` and friends

Each hook gets a sandboxed `$app` global exposing the app's record API, the
filter parser, the mailer, the realtime broker, and a fetch client (gated by
the JS-capability policy).

Sandbox limits (set per app, bounded by realm, bounded by master):
- CPU time per hook invocation
- Memory cap per VM
- Network egress allowlist
- Filesystem access (off by default)

---

## Dashboard & client SDKs

- **Dashboard**: SvelteKit SPA in `ui/` (same workspace, separate from
  the Rust crates). Built with `bun --cwd ui run build` to `ui/build/`,
  embedded into the `rustbase` binary via `include_dir!` in
  `rustbase-server/src/dashboard.rs`. Served at `/_/` with index-fallback
  for client-side routing. Navigates `Realm → App → Collection`; master
  admins also see a "System" tab. Dev iteration: `bun --cwd ui run dev`
  on :5173 with a vite proxy forwarding API paths to the Rust server on
  :8080. Runtime override: `RUSTBASE_DASHBOARD_PATH` points to any built
  directory and bypasses the embed.
- **REST API shape**: `/api/realms/<realm>/apps/<app>/collections/<name>/records[?filter=...]`
- **Client SDKs**: JS/TS first, then Dart, then Go. Idiomatic and ergonomic per language. Generated against an OpenAPI spec emitted by `rustbase-api`.

---

## Testing conventions

- Unit tests: in the same file, `#[cfg(test)] mod tests { ... }`.
- Integration tests: in `tests/` of each crate.
- All DB tests use `sqlite::memory:` — fresh DB per test, no Docker, sub-second suite.
- A shared test suite in `rustbase-db/src/testing.rs` exercises every public DB operation; CI runs it against both `:memory:` and a temp file to catch durability/WAL issues.
- Auto-clamp behavior has a dedicated property test suite: generate a random master config + N random realm configs + N random app configs all valid, then narrow a master bound and assert that every stored value is in-range afterwards and every change is in the audit log.

---

## Dev tooling

Git hooks are tracked under `.githooks/`. Wire them on first clone:

```sh
./scripts/install-hooks.sh
```

This sets `git config core.hooksPath .githooks`. The hooks run:

- **`pre-commit`** (fast, sub-5s): `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, arch greps (no `unwrap`/`expect` outside `#[cfg(test)]`, `rustbase-core` stays IO-free), and a no-AI-attribution scan of the staged diff.
- **`pre-push`** (slower, ~10–15s): `cargo test --workspace`, and `cargo audit` if installed (`cargo install cargo-audit`).

Bypass for one commit only with `git commit --no-verify` / `git push --no-verify`.

The same checks run on every PR via `.github/workflows/ci.yml`. The `release-build` job (`cargo build --workspace --release`) is gated on the check job passing.

`cargo audit`'s ignore list lives in `.cargo/audit.toml` — every entry carries a one-line rationale and must be re-evaluated on every dependency bump.

### Shared dev services

Local infrastructure that's reusable across apps — currently just MailHog — lives in `infra/docker-compose.yml`. Bring it up with:

```sh
docker compose -f infra/docker-compose.yml up -d
```

MailHog binds host ports `localhost:1025` (SMTP) and `localhost:8025` (web UI), and also joins the named `dev-shared` Docker network so other compose-managed apps can attach. See `infra/README.md` for the cross-app wiring recipe.

To exercise the `SmtpMailer` end-to-end against MailHog, opt into the ignored test:

```sh
cargo test -p rustbase-api smtp_mailer_delivers_to_mailhog -- --ignored --nocapture
```

---

## Technology choices (locked — do not change without updating this file)

| Concern | Choice | Rationale |
|---|---|---|
| Async runtime | `tokio` | Ecosystem standard |
| HTTP framework | `axum` 0.8+ | Tower ecosystem, no macros |
| Database | SQLite via `sqlx` | Single-file, embedded, zero-ops |
| SQL | `sqlx::query!` for static, hand-rolled parameterized builder for dynamic filters | One dialect → no need for `sea-query` |
| JS/TS runtime | `rquickjs` + `swc` | Fast, embedded, no Node.js dependency |
| Serialization | `serde` + `serde_json` | Standard |
| Error types | `thiserror` | Structured errors |
| Validation | `validator` | Field-level validation |
| JWT | `jsonwebtoken` | Widely used, well maintained |
| Password hashing | `argon2` | Current best practice |
| Filter parsing | `nom` | Zero-copy parser |
| File storage abstraction | `object_store` | Apache project, S3 + local |
| Concurrent map | `dashmap` | Lock-free reads for broker/pool maps |
| Dashboard UI | SvelteKit, prebuilt, embedded via `include_dir!` | Single-binary install |
| Config | `config` crate + env vars | 12-factor compatible |
| Replication | Litestream (optional) | SQLite → S3, no app changes |

---

## What agents must NOT do

- Do not add a new crate to the workspace without updating this file and ARCHITECTURE.md.
- Do not add `unwrap()` or `expect()` outside `#[cfg(test)]` blocks.
- Do not write raw SQL strings that interpolate user input — every value is a bound parameter.
- Do not bypass `AppCtx` / `RealmCtx` — there is no "admin mode" that skips the realm/app scope.
- Do not allow the master realm to be deleted; renaming is fine, deletion is not.
- Do not let a write succeed when its value violates the hierarchical policy chain — validation walks master → realm → app at write time.
- Do not make `rustbase-core` depend on any IO crate.
- Do not store binary file data in the database.
- Do not change the `FilterNode` AST without updating the SQL translator, the dashboard validator, and the JS/TS hook API surface.
- Do not change a `ConfigPolicy` field's kind (`Range` ↔ `Toggle` ↔ `EnumSet`) without a migration that maps existing values.
- Do not use `tokio::time::timeout` directly on DB driver futures (spawn + timeout the `JoinHandle`).
- Do not add synchronous blocking calls inside async functions (use `tokio::task::spawn_blocking`).
- Do not run JS/TS hooks outside the `rustbase-runtime` sandbox.
- Do not reintroduce a `DatabaseBackend` trait until there's a concrete second backend to motivate it.

---

## Bootstrap sequence

On server start, `rustbase-server` does the following in order:

1. Load config (file + env vars).
2. Verify `data/` exists (create if missing).
3. Open the system pool (`data/system.db`), run system migrations.
4. If no master realm exists, create it. If no master admin exists, mark the server as **uninitialized** and serve only the setup wizard at `/_/setup`.
5. Discover existing realms under `data/realms/` and run pending realm migrations for each.
6. For each realm, discover existing apps and run pending app migrations.
7. Initialize the realm and app pool managers (LRU caps).
8. Initialize the realtime broker.
9. Initialize the storage backend.
10. Initialize the JS/TS runtime; load hooks for each `(realm, app)`.
11. Optionally start Litestream sidecars.
12. Start the axum HTTP server.
13. Serve dashboard static assets at `/_/`.
14. Serve API at `/api/realms/<realm>/apps/<app>/...`.

---

## Project conventions

- Default deployment: a single binary + `data/` directory. `./rustbase` and you have a working server.
- First run: the dashboard prompts for the master admin credentials before anything else is accessible.
- Configuration: `rustbase.toml` in the working directory, overridable by `RUSTBASE_*` env vars.
- License: dual MIT + Apache-2.0 (`license = "MIT OR Apache-2.0"` in every `Cargo.toml`).
- Dashboard: SvelteKit SPA under `ui/`, built with `bun --cwd ui run build`. `rustbase-server/build.rs` runs the build automatically when needed; the static artifact is then embedded into the `rustbase` binary via `include_dir!`. Dev iteration: `bun --cwd ui run dev` on :5173 with a vite proxy to the Rust API on :8080. Runtime override: `RUSTBASE_DASHBOARD_PATH=<dir>` bypasses the embed.
- Client SDKs: separate repos, generated against the OpenAPI spec emitted by `rustbase-api`.
