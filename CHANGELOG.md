# Changelog

All notable changes to this project are documented here.

Format inspired by [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- **Dashboard records list: optimistic updates + bulk delete.**
  Single-row delete now drops the row from the table immediately and
  rolls back the snapshot if the DELETE comes back as an error.
  Inline edits paint into the table the moment the modal submits and
  revert if the PATCH fails, with the error surfacing inside the
  still-open modal. A new checkbox column lets the user fan-select
  rows; the header checkbox carries the standard tri-state
  (none / some / all on-page). When at least one row is selected a
  bulk-actions toolbar floats above the table with **Delete N** and
  **Clear**. The bulk delete dispatches one DELETE per row via
  `Promise.allSettled`, optimistically clears the visible rows, and
  on partial failure re-syncs from the server while keeping the
  failing IDs in the selection set. Pagination, filter changes, and
  collection navigation all clear the selection — rows that aren't
  visible can't communicate state.
- **Dashboard dark mode + a11y baseline.** Theme rune
  (`$lib/theme.svelte`) persists a 3-state choice — Auto / Light /
  Dark — in `localStorage`. `Auto` follows the OS-level
  `prefers-color-scheme` and re-evaluates when the OS preference
  changes. A new `ThemeToggle` button in the global header cycles
  through the three states. Tailwind's `dark:` variant is
  reconfigured for class-based dark mode and the component classes
  in `routes/layout.css` (`.btn-primary`, `.btn-secondary`,
  `.input`, `.card`, `.error-banner`, etc.) ship matching dark
  palettes.
  A11y baseline lands alongside: a keyboard-only "Skip to main
  content" link, a stylesheet-level focus-visible ring on every
  focusable element, ARIA labels on the global landmarks, and a
  `<main id="main-content" tabindex="-1">` target. Two new
  Playwright specs cover the theme persistence + skip-link round
  trip.
- New `Skeleton.svelte` component replaces the 12 hand-rolled
  `<p>Loading…</p>` placeholders across the dashboard with
  animated, dark-mode-aware skeleton rows. Tagged `role="status"
  aria-busy="true"` so screen readers announce the loading state.
- **Realtime gets server-side filters + a WebSocket transport.**
  - `FilterNode::matches(fields)` evaluates the same AST the SQL
    translator consumes against an in-memory record, with parity
    semantics for `Eq` / `Ne` / `Gt` / `Gte` / `Lt` / `Lte` /
    `Like` / `In` / `And` / `Or` / `Not`.
  - The SSE endpoint (`GET …/collections/:coll/events`) now accepts
    an optional `?filter=<expression>` and only forwards events
    whose record matches; `record_deleted` events always pass so
    subscribers can evict cached rows. For collections with a
    template access rule (e.g. `owner = @request.auth.id`), the
    rule is materialised against the subscribing principal and
    **intersected** with the client filter. Template-rule
    collections, which previously denied realtime outright, are
    now supported.
  - New WebSocket endpoint
    (`GET …/collections/:coll/events/ws`) — same auth, same
    `?filter=`, same JSON event payloads as SSE. Push-only; client
    frames are drained but never interpreted.
- **JS hook runtime** gains two new bridges on the `$app` global:
  - `$app.fetch(url, init?)` — synchronous outbound HTTP from a hook.
    Backed by a shared `reqwest::Client` with a 30 s timeout; the
    request is rejected with `Forbidden` before any network IO when
    the URL's host isn't on the workspace fetch allowlist
    (`[hooks.fetch].allowed_hosts` in `rustbase.toml`). Returns an
    object with `status`, `headers`, `text()`, and `json()`.
  - `$app.audit.write({action, target?, details?})` — append one row
    to the per-app audit log straight from a hook. Stored with
    `actor = "hook"` so the dashboard distinguishes operator events
    from user-initiated ones.
- `rustbase_runtime::AppHooksConfig` + `HookEngine::load_app_with` —
  full bridge bundle on one call. Existing `load_app` / `with_records*`
  signatures stay as-is for tests; production code (`apps.rs`,
  `hooks.rs`, `main.rs::load_all_hooks`) uses the new path.
- New concept page [`docs/concepts/write-amplification.md`](docs/concepts/write-amplification.md)
  — documents the per-pool fsync ceiling, the post-batching commit
  count for every hot auth path, and the explicit future-work list
  for audit + multi-step flows.
- **Supply-chain hardening.** Every release artefact now ships with
  provenance and a vulnerability paper trail:
  - Each tarball, the container image, and a workspace **CycloneDX
    1.5 SBOM** are signed keylessly with **Sigstore Cosign** (GitHub
    OIDC → Fulcio short-lived certs). `.sig` + `.pem` sidecars ride
    along with every artefact. Verification recipe in
    [`SECURITY.md`](SECURITY.md#verifying-release-artefacts).
  - **Trivy** scans the workspace filesystem on every PR
    (Rust + Bun lockfile transitives, Dockerfile mis-configs,
    secret leaks) and the released container image post-publish.
    Findings surface on the GitHub Security tab — `exit-code: 0`
    so a new transitive CVE doesn't block PRs / releases.
  - **CodeQL** static analysis of the dashboard
    (JavaScript / TypeScript / Svelte) on push to `main`, every PR,
    and weekly.
- **Playwright end-to-end smoke suite** in `ui/tests/e2e/` driven by
  `scripts/e2e-server.sh` (boots a release `rustbase` against a
  throw-away `data/` dir + the freshly-built dashboard, then runs the
  suite headless). One spec walks setup → login → workspace → app →
  collection. Run with `make e2e` after a one-off `make e2e-install`.
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
- **JWT signing now uses RS256 by default** with a deterministic `kid`
  derived from the SHA-256 of the public key. RSA-2048 keypair is
  generated once at first boot and persisted as PKCS#8 DER under
  `system.db._secrets`.
- **JWKS endpoint** at `/.well-known/jwks.json` and
  `/_/auth/jwks.json`. Returns `Content-Type:
  application/jwk-set+json`, `Cache-Control: public, max-age=3600`,
  and stays reachable pre-setup so external smoke probes can discover
  the key. Standard JWT libraries (jose, jsonwebtoken,
  oidc-client-ts) consume it without custom config.
- `rustbase-auth::JwtIssuer` is the new issuance/verification surface
  (`issue`, `verify`, `jwks`). HS256 tokens issued before the upgrade
  continue to verify until they expire on their own; the legacy HMAC
  key is kept as a verification-only fallback.

### Changed
- **Write amplification on the auth happy paths cut roughly in half.**
  Two new combinators in `rustbase_db::tokens` —
  `commit_user_login` (bumps `users.last_login` + inserts the
  refresh-token row in one transaction) and `rotate_refresh_token`
  (revokes the old + inserts the new in one transaction) — collapse
  the per-fsync cost of every successful login (password / OAuth /
  TOTP / email-OTP) and every refresh-rotation to a single commit
  each, down from two. The handler-side migrations are mechanical
  drop-in replacements; no semantic change to the API.

### Fixed
- **Dashboard CSP**: the strict `script-src 'self'` header we shipped
  in the security-layer-1 rollout was blocking SvelteKit's own
  SHA-hashed inline boot script, leaving the dashboard a blank page
  whenever `http.security_headers = true` (the default). The fix
  moves the dashboard's CSP onto a `<meta http-equiv>` tag emitted
  by SvelteKit (`kit.csp.mode = 'hash'`), which auto-hashes every
  inline script per build, and drops the server-side CSP header
  entirely (JSON API responses don't need CSP).

### Changed — BREAKING

- **Concept renamed: `Realm` → `Workspace`.** Every public surface
  follows: REST routes (`/api/realms/...` → `/api/workspaces/...`),
  dashboard URLs (`/_/realms/...` → `/_/workspaces/...`), on-disk
  layout (`data/realms/<id>/realm.db` → `data/workspaces/<id>/workspace.db`),
  DB tables (`realms` → `workspaces`, `realm_admins` →
  `workspace_admins`), JSON config keys (`realm_pool_cap` →
  `workspace_pool_cap`), error codes (`realm_not_found` →
  `workspace_not_found`), JWT claims (`realm` → `workspace`), Rust
  types (`RealmId`, `RealmAdmin`, `RealmCtx`, `RealmPoolManager` →
  `Workspace*`), and migration constants (`REALM_MIGRATIONS` →
  `WORKSPACE_MIGRATIONS`).
- **End-user identity moves from per-app to per-workspace.** A single
  `(email, workspace)` pair is one identity across every app in that
  workspace. Concretely:
    - User tables (`users`, `oauth_providers`, `user_oauth_links`,
      `_email_verifications`, `_password_resets`, `_email_otps`,
      `_user_totp`, `_mfa_challenges`, `_oauth_states`) move from
      app-scope `data.db` to workspace-scope `workspace.db`.
    - Every auth route loses its `/apps/:app/` segment. New paths:
      `POST /api/workspaces/:workspace/auth/users/login`,
      `…/auth/users/register`, `…/auth/users/refresh`,
      `…/auth/verify-email/{request,confirm}`,
      `…/auth/password-reset/{request,confirm}`,
      `…/auth/otp/{request,login}`,
      `…/auth/totp/{enroll,confirm,disable}`,
      `…/auth/users/login/totp`,
      `…/auth/oauth/{provider}/{authorize,callback}`,
      `…/auth/oauth/providers/[/{provider}]`.
    - Admin user management routes likewise drop the `/apps/:app/`
      segment: `GET/PATCH/DELETE /api/workspaces/:workspace/users[/{id}…]`.
    - User access tokens lose the `app` claim. The per-app target
      now comes from the URL path on data routes
      (`/api/workspaces/:workspace/apps/:app/...`) rather than the
      token claim.
    - `PrincipalAuth::require_user_in_app(workspace, app)` →
      `require_user_in_workspace(workspace)`; `user_app()` removed.
    - User-lifecycle hooks (`onUserBeforeLogin`,
      `onUserAfterLogin`, `onUserAfterRegister`) fan out across
      every app in the workspace until workspace-scoped hook
      loading lands — any app's hook can veto the login.
- No automatic data migration: existing 0.1.x installs need to
  point at a fresh `data/` directory or rename the layout manually
  before upgrading. The maintainer's own dev DB was bootstrapped from
  scratch.
- **PKCE (RFC 7636) on every OAuth flow.** `/authorize` mints a
  32-byte `code_verifier`, persists it alongside the CSRF state,
  sends `code_challenge=S256(verifier)` + `code_challenge_method=S256`
  to the upstream provider; `/callback` replays the verifier on the
  token exchange. New `_oauth_states.code_verifier` column added via
  app-scoped migration `20260604_000001_oauth_pkce`.
- **Dashboard session moved to `HttpOnly` cookies.** Login + refresh
  responses set `rb_at` (Path `/`) and `rb_rt` (Path `/_/auth`) with
  `HttpOnly; SameSite=Strict` (and `Secure` when
  `[http].cookie_secure = true`, the default). The dashboard SPA
  removes JWT/refresh tokens from `localStorage`; the React-y identity
  blob it keeps is no longer secret. New `POST /_/auth/logout`
  endpoint clears both cookies and revokes the refresh token.
  `AdminAuth` / `PrincipalAuth` accept `rb_at` as a fallback for
  `Authorization: Bearer …` so SDK clients are unaffected.

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
  no `_app/` prefix), so deep links like `/_/workspaces/acme/apps/web`
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
