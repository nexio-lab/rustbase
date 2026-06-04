# Architecture

RustBase is split into one workspace with eight Rust crates plus a SvelteKit dashboard. The crate boundaries are real boundaries — each one has a single responsibility and a one-direction dependency graph.

## Crate map

| Crate | Purpose |
|---|---|
| `rustbase-core` | IO-free domain types: `WorkspaceId`, `AppId`, `Record`, `Schema`, `FilterNode`, `ConfigPolicy`, error enum, filter parser |
| `rustbase-db` | SQLite layer: system / workspace / app pools, migrations, CRUD, filter → SQL, cascade-delete, auto-clamp engine |
| `rustbase-auth` | JWT, argon2, OAuth2, OTP, master / workspace / app admin model |
| `rustbase-realtime` | In-process pub/sub broker |
| `rustbase-storage` | Local + S3 file storage via `object_store` |
| `rustbase-runtime` | Embedded JS/TS runtime, hook dispatch, sandboxing |
| `rustbase-api` | axum handlers (REST, SSE, WebSocket), `ApiError → IntoResponse` |
| `rustbase-server` | Binary: config, bootstrap, setup wizard, embedded dashboard |

## Dependency rules

- `rustbase-core` depends on **no IO crate** — no `tokio`, no `sqlx`, no `axum`.
- Every other crate depends on `rustbase-core`.
- `rustbase-api` depends on `rustbase-db`, `rustbase-auth`, `rustbase-realtime`, `rustbase-storage`, `rustbase-runtime`.
- `rustbase-server` depends on `rustbase-api`.

That keeps the domain types testable with `cargo test -p rustbase-core --no-default-features` and prevents accidental coupling.

## Request flow

```
┌─────────────┐   HTTP   ┌──────────────┐
│   client    │──────────│ rustbase-api │
└─────────────┘          │   (axum)     │
                         └──────┬───────┘
                                │
            ┌───────────────────┼──────────────────┐
            ▼                   ▼                  ▼
   ┌─────────────────┐ ┌────────────────┐ ┌──────────────────┐
   │  rustbase-db    │ │ rustbase-auth  │ │ rustbase-runtime │
   │ (sqlx + SQLite) │ │   (JWT, hash)  │ │    (QuickJS)     │
   └────────┬────────┘ └────────────────┘ └──────────┬───────┘
            │                                        │
            ▼                                        ▼
   ┌─────────────────┐                      ┌──────────────────┐
   │   data/*.db     │                      │  data/hooks/...  │
   └─────────────────┘                      └──────────────────┘
```

Lifecycle hooks publish to the **realtime broker** after a successful DB write; SSE/WebSocket subscribers consume from the same broker.

## Bootstrap sequence

On server start, `rustbase-server` does the following in order:

1. Load config (file + env vars).
2. Verify `data/` exists (create if missing).
3. Open the system pool (`data/system.db`), run system migrations.
4. If no master workspace exists, create it. If no master admin exists, mark the server as **uninitialized** and serve only the setup wizard at `/_/setup`.
5. Discover existing workspaces under `data/workspaces/` and run pending workspace migrations for each.
6. For each workspace, discover existing apps and run pending app migrations.
7. Initialize the workspace and app pool managers (LRU caps).
8. Initialize the realtime broker.
9. Initialize the storage backend.
10. Initialize the JS/TS runtime; load hooks for each `(workspace, app)`.
11. Optionally start Litestream sidecars.
12. Start the axum HTTP server, layered with the setup gate and the trace middleware.

## Filter parser

`rustbase-core` contains a `nom`-based parser that produces a `FilterNode` AST:

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

The parser is IO-free and dialect-agnostic. `rustbase-db` translates a `FilterNode` into a parameterized SQL `WHERE` clause via `filter_to_sql` — no string interpolation of user input, every literal becomes a bound parameter.

The same AST is reused by:

- The dashboard for client-side validation.
- The per-collection access-rules engine.
- The JS/TS hooks API for `$app.records.findRecordsByFilter(...)`.

## Error handling

One error enum per crate, `thiserror`-derived. Boundaries map foreign errors into the local enum:

- `rustbase-db` maps `sqlx::Error` → `CoreError`.
- `rustbase-api` maps `CoreError` → `ApiError` which implements `IntoResponse`.

The HTTP layer returns a uniform `{code, message}` JSON shape for every error, mapped to a sensible status code (404 for not-found, 403 for forbidden, etc.).

## Testing

- Unit tests live in-file under `#[cfg(test)] mod tests`.
- Integration tests live in `tests/` per crate.
- All DB tests use `sqlite::memory:` — fresh DB per test, no Docker, sub-second suite.
- A shared test suite in `rustbase-db/src/testing.rs` exercises every public DB operation; CI runs it against both `:memory:` and a temp file to catch durability/WAL issues.
- Auto-clamp behavior has a dedicated property test suite.

CI runs `cargo fmt --check`, `cargo clippy -D warnings`, the full test suite, and an architecture grep (no `unwrap`/`expect` outside tests; `rustbase-core` stays IO-free) on every PR.
