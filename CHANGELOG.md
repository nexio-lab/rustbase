# Changelog

All notable changes to this project are documented here.

Format inspired by [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning follows [Semantic Versioning](https://semver.org/).

## [0.1.1](https://github.com/pjonaszik/rustbase/compare/v0.1.0...v0.1.1) (2026-06-02)


### Fixed

* **dashboard:** honour paths.base under the /_/ mount (closes [#17](https://github.com/pjonaszik/rustbase/issues/17)) ([a51b7f5](https://github.com/pjonaszik/rustbase/commit/a51b7f57f0f5313acf18060604755330dcc0d5ac))


### Build

* **deps-dev:** bump prettier-plugin-svelte from 3.5.2 to 4.0.1 in /ui ([#7](https://github.com/pjonaszik/rustbase/issues/7)) ([c57569b](https://github.com/pjonaszik/rustbase/commit/c57569b7370da96d976fc74a263f2cfb31f5eece))
* **deps-dev:** bump prettier-plugin-tailwindcss in /ui ([#6](https://github.com/pjonaszik/rustbase/issues/6)) ([ed27bc8](https://github.com/pjonaszik/rustbase/commit/ed27bc8362072f50f1dfac94e087cabd0a02cd5e))
* **deps:** bump actions/checkout from 4 to 6 ([#3](https://github.com/pjonaszik/rustbase/issues/3)) ([d31cb55](https://github.com/pjonaszik/rustbase/commit/d31cb55bbe2d9e2be9c1ff7d1afbb12636cbdd32))
* **deps:** bump actions/configure-pages from 5 to 6 ([#5](https://github.com/pjonaszik/rustbase/issues/5)) ([5959e56](https://github.com/pjonaszik/rustbase/commit/5959e56fa1bef2d3c350f6914952f7a79b899e37))
* **deps:** bump actions/deploy-pages from 4 to 5 ([#4](https://github.com/pjonaszik/rustbase/issues/4)) ([9746448](https://github.com/pjonaszik/rustbase/commit/974644816fb9370d4de37c0e3ae119d10126d2d3))
* **deps:** bump actions/download-artifact from 4 to 8 ([#14](https://github.com/pjonaszik/rustbase/issues/14)) ([30f5a3a](https://github.com/pjonaszik/rustbase/commit/30f5a3aa5820d3d04a1647452687315f23534210))
* **deps:** bump actions/upload-artifact from 4 to 7 ([#15](https://github.com/pjonaszik/rustbase/issues/15)) ([f8ed2b9](https://github.com/pjonaszik/rustbase/commit/f8ed2b9c09983c6ce8baa1e0e00f9c0512a2448e))
* **deps:** bump actions/upload-pages-artifact from 3 to 5 ([#1](https://github.com/pjonaszik/rustbase/issues/1)) ([4b2e33f](https://github.com/pjonaszik/rustbase/commit/4b2e33fe26fd9e40a9b6c2ec5c26e2434664074c))
* **deps:** bump config from 0.14.1 to 0.15.23 ([#10](https://github.com/pjonaszik/rustbase/issues/10)) ([0d952c4](https://github.com/pjonaszik/rustbase/commit/0d952c478b2a26c638cc59f7aab5d36c5dd8ca0b))
* **deps:** bump rquickjs from 0.9.0 to 0.11.0 ([#9](https://github.com/pjonaszik/rustbase/issues/9)) ([192479b](https://github.com/pjonaszik/rustbase/commit/192479b723b37283b6f09260b22ddfda298ad566))
* **deps:** bump softprops/action-gh-release from 2 to 3 ([#2](https://github.com/pjonaszik/rustbase/issues/2)) ([c82f561](https://github.com/pjonaszik/rustbase/commit/c82f56159474496b2076c78056fbb2fe094e5568))
* **deps:** bump the patch-updates group with 2 updates ([#8](https://github.com/pjonaszik/rustbase/issues/8)) ([8bec598](https://github.com/pjonaszik/rustbase/commit/8bec59884527c9ab4015ff6d054b2271c607c043))
* **deps:** bump validator from 0.19.0 to 0.20.0 ([#12](https://github.com/pjonaszik/rustbase/issues/12)) ([c6a1a37](https://github.com/pjonaszik/rustbase/commit/c6a1a37644a51b52fdfc79276581f951926aeaf0))


### CI

* **release-please:** add workflow_dispatch trigger ([a053f2c](https://github.com/pjonaszik/rustbase/commit/a053f2cb5c4b20ca3ae44826f784d6c70a93a8d1))
* **release-please:** switch to simple+extra-files; release-please rust strategy can't parse workspace-inherited versions ([f105e50](https://github.com/pjonaszik/rustbase/commit/f105e502ba0161153d9a716e04eb1823f1098c1c))
* **release:** wire release-please for automated version bumps + changelog ([9e137f5](https://github.com/pjonaszik/rustbase/commit/9e137f5113f8ba514f897251b9b188119cc28340))

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
- README header screenshot now showcases the real sign-in page of the
  embedded dashboard.

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
