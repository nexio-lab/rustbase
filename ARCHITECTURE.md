# Architecture

A short, durable overview for contributors. Anything user-facing belongs in
the [docs site](https://pjonaszik.github.io/rustbase/); this file is what a
new maintainer needs to find their way around the codebase.

## Hierarchy

```
System
  └── Realm  (organization boundary — admins live here)
        └── App  (data product — collections, records, files, end-users, OAuth live here)
```

- One `system.db` per RustBase instance.
- One `realm.db` per realm, holding the apps registry + admin tiers.
- One `data.db` per app, holding collections / records / users / OAuth.

Full mental model: [docs / concepts / mental-model](https://pjonaszik.github.io/rustbase/concepts/mental-model).

## Crates

| Crate | Purpose | Depends on |
|---|---|---|
| `rustbase-core` | IO-free domain types (`RealmId`, `AppId`, `Schema`, `FilterNode`, error enum, filter parser). | nothing IO |
| `rustbase-db` | SQLite layer: system / realm / app pools, migrations, CRUD, `FilterNode → SQL`, cascade-delete, auto-clamp. | `core` |
| `rustbase-auth` | JWT (HS256), argon2, AES-GCM KEK for at-rest OAuth secrets, TOTP, OAuth2 client. | `core` |
| `rustbase-realtime` | In-process pub/sub broker over `tokio::sync::broadcast`. | `core` |
| `rustbase-storage` | File backend abstraction (`object_store`): local disk + S3. | `core` |
| `rustbase-runtime` | Embedded JS/TS hook runtime (`rquickjs`), sandboxing, dispatch. | `core` |
| `rustbase-api` | axum handlers — REST, SSE, dashboard mount. `ApiError → IntoResponse`. | all of the above |
| `rustbase-server` | The binary. Config, bootstrap, embedded SvelteKit dashboard. | `api` |

Dependency rules:

- `rustbase-core` depends on no IO crate. The architectural grep in
  `.githooks/pre-commit` enforces this.
- Every other crate depends on `rustbase-core`.
- `rustbase-api` depends on all foundational crates.
- `rustbase-server` only depends on `rustbase-api`.

## Storage layout

```
data/
  system.db                           # realms registry, master admins, master audit
  realms/
    <realm_id>/
      realm.db                        # apps, realm/app admins, admin refresh tokens, realm audit
      storage/                        # realm-level files (rarely used)
      apps/
        <app_id>/
          data.db                     # collections, records, users, oauth,
                                      # user refresh tokens, verifications,
                                      # password resets, OTPs, TOTP, MFA,
                                      # file metadata, app audit
          storage/                    # app-level files (binary blobs)
  hooks/
    <realm_id>/<app_id>/              # JS/TS hook source files (*.js, *.ts)
```

Per-connection PRAGMAs (set on every pool):

```
journal_mode = WAL
foreign_keys = ON
busy_timeout = 5000
synchronous  = NORMAL
```

Pool management lives in `rustbase-db::pool`:

| Pool | Key | Cap (default) | Eviction |
|---|---|---|---|
| System | — | always open | n/a |
| Realm | `RealmId` | 32 | LRU |
| App | `(RealmId, AppId)` | 64 | LRU |

## Filter parser

`rustbase-core::filter` exposes a `nom`-based parser that produces a
`FilterNode` AST. The translator in `rustbase-db::filter_to_sql` turns
that AST into a parameterized SQL `WHERE` clause. Every literal becomes
a bound parameter — no string interpolation of user input.

The same AST is reused by:

- Dashboard client-side validation.
- Per-collection access rules (`templates` like `@user.id == owner`).
- The JS/TS hook API surface (`$app.records.findRecordsByFilter`).

## Tokens

| Role | `realm` claim | `app` claim | Stored where |
|---|---|---|---|
| `master_admin` | — | — | system.db |
| `realm_admin` | yes | — | realm.db |
| `app_admin` | yes | yes | realm.db |
| `user` | yes | yes | app.db |

Access tokens are HS256 JWTs with a 15-minute default TTL. Refresh tokens
are opaque random strings stored in the matching scope's
`_refresh_tokens` table, exchanged at the matching `/auth/refresh`
endpoint. Rotation-on-use: every refresh revokes the presented token and
issues a new pair.

## Error handling

One error enum per crate, derived via `thiserror`. `rustbase-db` maps
`sqlx::Error` into `CoreError` at the boundary. `ApiError` in
`rustbase-api` implements `IntoResponse` and maps `CoreError` variants
to HTTP status codes.

No `unwrap()` / `expect()` outside `#[cfg(test)]` blocks. The pre-commit
hook enforces this.

## Bootstrap sequence

`rustbase-server::main` on start:

1. Load config (file + env vars).
2. Open the system pool, run system migrations.
3. Ensure the master realm row exists.
4. `ensure_seed_master_admin` (idempotent) inserts the `admin` row with
   NULL password if missing. The setup wizard at `POST /_/setup` finalizes
   the bootstrap by setting that password.
5. Discover existing realms / apps and run pending migrations.
6. Initialize the realm and app pool managers (LRU caps).
7. Initialize realtime broker.
8. Initialize storage backend (local or S3).
9. Initialize the JS/TS runtime; load hooks for every loaded `(realm, app)`.
10. Optionally start Litestream sidecars.
11. Start the axum HTTP server.

## Testing conventions

- Unit tests: in the same file, `#[cfg(test)] mod tests { ... }`.
- Integration tests: in `tests/` of each crate.
- All DB tests use `sqlite::memory:` — fresh DB per test, no Docker.
- A shared test suite in `rustbase-db/src/testing.rs` exercises every
  public DB operation.
- The auto-clamp engine has property-based coverage in
  `rustbase-db/tests/`.

## What contributors must NOT do

- Add a new crate to the workspace without updating this file and the docs.
- Add `unwrap()` / `expect()` outside `#[cfg(test)]`.
- Write raw SQL strings that interpolate user input — every value is a
  bound parameter.
- Bypass `AppCtx` / `RealmCtx` — there is no "admin mode" that skips the
  realm / app scope.
- Allow the master realm to be deleted; renaming is fine, deletion is not.
- Make `rustbase-core` depend on any IO crate.
- Store binary file data in the database.
- Change the `FilterNode` AST without updating the SQL translator, the
  dashboard validator, and the JS/TS hook API surface.
- Run JS/TS hooks outside the `rustbase-runtime` sandbox.

See [CONTRIBUTING.md](CONTRIBUTING.md) for dev setup and PR conventions.
