# Changelog

All notable changes to this project are documented here.

Format inspired by [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- **Brand: project renamed to RustBase.** New tagline: _"Multi-tenant
  backend. Single binary. Real isolation."_ The on-disk layout, crate
  names, and config keys are unchanged; only public-facing wording
  moved.
- New positioning page (`docs/concepts/positioning.md`) — explicit
  "who this is for" / "who it is not for" / "what multi-tenant means
  here precisely" / "when to outgrow RustBase."
- New comparison page (`docs/guide/comparison.md`) — RustBase vs
  PocketBase / Supabase / Appwrite, feature-by-feature plus a
  decision matrix.
- Deployment guide rewritten end to end: Hetzner sizing, hardened
  systemd unit, Caddy + Let's Encrypt with security headers, Nginx
  alternative, tarball-backup timer, Litestream sidecar, upgrade flow,
  hardening checklist.
- **Security layer 1 — defaults on:**
  - Per-IP token-bucket rate limit at the entry layer (50 r/s, 100
    burst by default) via `tower_governor`. Rejected with `429
    too_many_requests` + `Retry-After`. Tunable under `[rate_limit]`.
  - Per-subject auth lockout shared across password / TOTP /
    email-OTP. 5 failures inside 5 min → 5 min lockout, returned as
    `429` + `Retry-After`. Tunable under `[lockout]`.
  - Conservative default-on security headers (HSTS, X-Content-Type-
    Options, Referrer-Policy, X-Frame-Options, baseline CSP,
    Permissions-Policy). Tunable / disable under `[http]`.
  - CORS allowlist; empty default = same-origin only. Tunable under
    `[cors]`.
  - HTTP request body cap (`[http].max_body_bytes`, default 8 MiB).
  - Audit rows: `login_success` / `login_failed` / `login_locked` for
    every auth flow.
- New error variant `CoreError::TooManyRequests { retry_after_secs }`
  → HTTP 429 with `Retry-After` header. Reference docs updated.

### Changed
- README hero copy and badge row aligned with the new positioning.
- `rustbase.toml.example` documents every new section.
- `docs/guide/configuration.md`: full reference now includes `[http]`,
  `[cors]`, `[rate_limit]`, `[lockout]`.

## [0.1.1] — 2026-06-03

### Added
- Multi-arch Docker image published to `ghcr.io/pjonaszik/rustbase` on
  every release tag (linux/amd64 + linux/arm64).
- Root-level `Dockerfile` for reproducible builds + local `docker build`.
- `ARCHITECTURE.md` summarising the crate map, hierarchy, and bootstrap
  for new contributors.
- `ROADMAP.md` with the public sketch of v0.2 → v1.0.
- Logo (`docs/public/logo.svg`) and favicons (32px / 192px / SVG) in
  RustBase orange.
- Social-preview image (`docs/public/social-preview.png`, 1280×640)
  wired into the docs `og:image` / Twitter card meta tags.
- README header now embeds a real dashboard sign-in screenshot.
- New GitHub label taxonomy: `type:*`, `scope:*`, `status:*`, `needs:*`.
- Welcome post pinned in GitHub Discussions (Announcements category).
- Dashboard smoke job in `ci.yml` that asserts asset prefixes, runtime
  `paths.base`, GET fallback on `/_/setup` / admin login / refresh, deep
  links, and that POST `/healthz` stays an honest 405.

### Changed
- README badge row reorganised; live screenshot replaces the previous
  text-only intro.

### Fixed (#17 — dashboard SPA mount)
- `rustbase-server/build.rs` now sets `VITE_BASE=/_` when invoking
  `bun run build`, so the SvelteKit static build embeds the correct
  `paths.base` at both the asset-path level (`/_/_app/...`) and the
  runtime config level (`__sveltekit_xxx.base = "/_"`).
- `crates/rustbase-server/src/main.rs` attaches a
  `method_not_allowed_fallback` to the merged router: `GET` requests
  under `/_/` that hit a POST-only API route (e.g. `/_/setup`,
  `/_/auth/admin/login`, `/_/auth/refresh`) fall through to the dashboard
  SPA shell so direct navigation + hard refresh work. Other 405s stay
  honest 405s — the rule is path-prefixed.
- `crates/rustbase-server/src/dashboard.rs::asset` falls back to
  `index.html` for paths that look like client-side routes (no extension,
  no `_app/` prefix), so deep links like `/_/realms/acme/apps/web`
  reload cleanly.
- `ui/src/lib/nav.ts` introduces a `goto` helper that prefixes
  SvelteKit's configured `base` to absolute-path hrefs. Every
  `+page.svelte` and `+layout.svelte` now navigates through this helper,
  so embedded-mount navigation no longer escapes the `/_/` prefix.

### Build
- Dependabot bumps: `download-artifact` 4 → 8, `upload-artifact` 4 → 7,
  `actions/checkout` 4 → 6, `actions/configure-pages` 5 → 6,
  `actions/deploy-pages` 4 → 5, `actions/upload-pages-artifact` 3 → 5,
  `softprops/action-gh-release` 2 → 3, `rquickjs` 0.9 → 0.11,
  `validator` 0.19 → 0.20, `config` 0.14 → 0.15, `prettier-plugin-svelte`
  3.5 → 4.0, `prettier-plugin-tailwindcss` 0.7 → 0.8.

[Unreleased]: https://github.com/pjonaszik/rustbase/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/pjonaszik/rustbase/compare/v0.1.0...v0.1.1

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

[Unreleased]: https://github.com/pjonaszik/rustbase/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/pjonaszik/rustbase/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/pjonaszik/rustbase/releases/tag/v0.1.0
