# Changelog

All notable changes to this project are documented here.

Format inspired by [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- Multi-arch Docker image published to `ghcr.io/pjonaszik/rustbase` on
  every release tag (linux/amd64 + linux/arm64).
- Root-level `Dockerfile` for reproducible builds + local `docker build`.
- `ARCHITECTURE.md` summarising the crate map, hierarchy, and bootstrap
  for new contributors.
- `ROADMAP.md` with the public sketch of v0.2 → v1.0.
- Logo (`docs/public/logo.svg`) and favicons (32px / 192px / SVG) in
  RustBaas orange.
- Social-preview image (`docs/public/social-preview.png`, 1280×640)
  wired into the docs `og:image` / Twitter card meta tags.
- README header now embeds the docs landing screenshot for a faster
  visual read.

### Changed
- README badge row reorganised; live screenshot replaces the previous
  text-only intro.

### Known issues (to be addressed in v0.2)
- Dashboard SPA does not consistently honour `paths.base = "/_"`: the
  layout route guard redirects to bare `/login` instead of `/_/login`,
  producing a 404 on hard refresh / direct deep-links. Workaround:
  always enter via the `/_/` root and let client-side routing take over.
  Tracked at <https://github.com/pjonaszik/rustbase/issues>.

## [0.1.0] — 2026-05-27

First public release. The shape of the API and on-disk layout is stable enough
to commit to, but everything is `0.x` — breaking changes are still possible
between minors until v1.0.

### Added
- **Three-level tenancy:** `System → Realm → App`. Master admins manage realms;
  realm admins manage apps in their realm; app admins are scoped to one app.
- **End-user pool per app.** Users, OAuth provider config, refresh tokens, and
  every auxiliary auth table (email verifications, password resets, OTPs,
  TOTP, MFA challenges) live in each app's `data.db`. A user registered in
  `acme/mobile` is a different identity than the same email in `acme/web`.
- **Auth flows:** email + password, email OTP (passwordless), TOTP second
  factor, OAuth2 / OIDC (Google, GitHub, Microsoft presets shipped). All
  flows under `/api/realms/:realm/apps/:app/auth/...`.
- **Auto-seeded master admin.** First boot inserts an `admin` master-admin
  row with a NULL password; the setup wizard (`POST /_/setup` with
  `{password}`) finalizes initialization. Login is `{username, password}`.
- **JWT tokens with explicit scope claims.** Master admin tokens carry no
  scope. Realm-admin tokens carry `realm`. App-admin and end-user tokens
  carry both `realm` and `app`. 15-min default access TTL, 30-day refresh
  with rotation-on-use.
- **Collections engine.** `base`, `auth`, and (planned) `view` kinds. Field
  types: text, number, bool, json, datetime, email, url, file, relation.
- **Filter parser.** Boolean expressions with `&&`, `||`, `!`, comparison
  operators, `~` LIKE, `?=` IN, and template placeholders. Compiles to
  parameterized SQL — every literal is bound.
- **Access rules per collection action.** `list`, `get`, `create`, `update`,
  `delete`, each with its own template or filter expression.
- **Realtime broker.** SSE and (planned) WebSocket subscriptions per
  collection. Post-mutation hooks publish to the broker.
- **File storage.** Local disk or any S3-compatible bucket via `object_store`.
  Metadata in `data.db`, blobs in `storage/`.
- **JS/TS hooks.** Embedded QuickJS runtime via `rquickjs`. Lifecycle hooks
  (`onRecord{Before,After}{Create,Update,Delete}`), user-lifecycle hooks
  (`onUser{BeforeLogin,AfterLogin,AfterRegister}`, app-scoped), custom HTTP
  routes (`$app.routerAdd`), cron jobs (`$app.cron`), mailer hooks. Sandboxed
  with hierarchical CPU / memory / network / FS policies.
- **Hierarchical policy engine.** Master sets bounds; realm tightens; app
  picks a value inside both. Auto-clamp when a parent narrows, with an audit
  entry written to every affected scope.
- **Audit log per scope.** Append-only `audit_log` table in each `*.db`.
- **Embedded SvelteKit dashboard** at `/_/`. Master-admin setup wizard,
  realm / app / collection / record CRUD, hooks editor, files browser,
  per-scope audit and policies, users + OAuth tabs per app.
- **VitePress documentation** at <https://pjonaszik.github.io/rustbase/>.
- **CI**: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`,
  `cargo audit` on every PR. Release build job on every push to `main`.

### Crates published
- `rustbase-core` — IO-free domain types + filter parser.
- `rustbase-db` — SQLite layer (system / realm / app pools, migrations).
- `rustbase-auth` — JWT, argon2, AES-GCM KEK, TOTP, OAuth2.
- `rustbase-realtime` — in-process pub/sub broker.
- `rustbase-storage` — local + S3 via `object_store`.
- `rustbase-runtime` — QuickJS-based hook runtime.
- `rustbase-api` — axum handlers (REST, SSE).
- `rustbase-server` — the binary.

[Unreleased]: https://github.com/pjonaszik/rustbase/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/pjonaszik/rustbase/releases/tag/v0.1.0
