# Handoff: feature/users-per-app

The two-part refactor described below is now **complete**. Tests pass workspace-wide (419 passed, 0 failed, 3 ignored) and `cargo clippy --workspace --all-targets -- -D warnings` is clean.

## Two refactors, one branch

### Part A — master admin auto-seed

**Goal:** On first boot, RustBaas auto-creates a master admin row with `username = "admin"` and `password_hash = NULL`. The setup wizard accepts only `{password}` and writes the hash. Master admin login uses `{username, password}`; the realm admins and end-users keep email-based login.

**Status:**

- ✅ `master_admins` schema has `username TEXT NOT NULL UNIQUE`; `email` and `password_hash` are nullable. [crates/rustbase-db/src/migrations.rs](crates/rustbase-db/src/migrations.rs).
- ✅ `MasterAdmin` + helpers in [crates/rustbase-db/src/admins.rs](crates/rustbase-db/src/admins.rs):
    - `ensure_seed_master_admin(pool)` — idempotent, inserts the `admin` row.
    - `find_master_admin_by_{username,id}`.
    - `set_master_admin_password(pool, id, hash)`.
    - `master_admin_is_initialized(pool)` — true iff any admin has a non-NULL `password_hash`.
- ✅ `POST /_/setup` takes `{password}` and updates the seeded row. [crates/rustbase-api/src/setup.rs](crates/rustbase-api/src/setup.rs).
- ✅ `POST /_/auth/admin/login` uses `MasterLoginRequest { username, password }` and returns `MasterAdminPublic { id, username, email: Option, name }`. [crates/rustbase-api/src/auth/login.rs](crates/rustbase-api/src/auth/login.rs).
- ✅ `rustbase-server/src/main.rs` bootstrap calls `ensure_seed_master_admin` after the master realm is created. The setup gate keys off `master_admin_is_initialized` (so the seeded row does not unblock it). [crates/rustbase-server/src/main.rs](crates/rustbase-server/src/main.rs).
- ✅ `/healthz` reports `initialized` from `master_admin_is_initialized`. [crates/rustbase-api/src/health.rs](crates/rustbase-api/src/health.rs).
- ✅ Dashboard `/setup` collects only the password. [ui/src/routes/setup/+page.svelte](ui/src/routes/setup/+page.svelte).
- ✅ Dashboard `/login` is `{username, password}` (default `admin`). [ui/src/routes/login/+page.svelte](ui/src/routes/login/+page.svelte).
- ✅ `MasterAdmin` API type carries `username: string` + `email: string | null`. [ui/src/lib/api.ts](ui/src/lib/api.ts).
- ✅ Master-admin refresh path stays unchanged; the response embeds `MasterAdminPublic` already.

### Part B — users live per-app

**Goal:** End-users live in each app's `data.db`, not in the realm. Auth endpoints become `/api/realms/<realm>/apps/<app>/auth/users/...`. OAuth providers + every user-bound auxiliary table moved with them. Realm admins keep cross-app reach (their token covers every app in the realm); app admins are scoped to one app.

**Status:**

- ✅ Migrations moved in [crates/rustbase-db/src/migrations.rs](crates/rustbase-db/src/migrations.rs):
    - `APP_MIGRATIONS` now owns `users`, `oauth_providers`, `user_oauth_links`, `_email_verifications`, `_password_resets`, `_email_otps`, `_oauth_states`, `_user_totp`, `_mfa_challenges`, and a per-app `_refresh_tokens`.
    - `REALM_MIGRATIONS` keeps `apps`, `realm_admins`, `app_admins`, the admin-only `_refresh_tokens`, `policies`, `audit_log`.
- ✅ `rustbase-db` tests updated. The new app-system tables (`users`, OAuth, etc.) are also reserved in `RESERVED_COLLECTION_IDS` so a developer can't create a collection that shadows the system table. [crates/rustbase-db/src/collections.rs](crates/rustbase-db/src/collections.rs).
- ✅ Every handler in `rustbase-api/src/auth/{login,register,refresh,verify_email,password_reset,email_otp,totp,oauth,oauth_admin}.rs` takes `Path<(String, String)>` (realm, app), checks `require_app_exists`, and uses `state.apps.pool_for(&realm_id, &app_id)`.
- ✅ The shared `require_app_exists` helper lives in [crates/rustbase-api/src/auth/mod.rs](crates/rustbase-api/src/auth/mod.rs).
- ✅ Admin user-management endpoints moved to `/api/realms/:realm/apps/:app/users/...` and use `require_app_access`. [crates/rustbase-api/src/users.rs](crates/rustbase-api/src/users.rs).
- ✅ Hook dispatch for `onUser{Before,After}Login` and `onUserAfterRegister` is now app-scoped — only the target app's hooks fire. [crates/rustbase-runtime/src/lib.rs](crates/rustbase-runtime/src/lib.rs).
- ✅ `TokenRole::User` JWTs carry both `realm` and `app`. `PrincipalAuth` gained `user_app()` + `require_user_in_app()`. [crates/rustbase-api/src/auth/extract.rs](crates/rustbase-api/src/auth/extract.rs).
- ✅ Router paths in [crates/rustbase-api/src/router.rs](crates/rustbase-api/src/router.rs) include the `/apps/{app}/` segment for every end-user, OAuth, and admin-user endpoint. Admin paths (`/_/auth/*`, `/api/realms/{realm}/auth/admin/login`, `/api/realms/{realm}/admins`, `/api/realms/{realm}/audit`, `/api/realms/{realm}/policies`) stay where they were.
- ✅ Tests: 133/133 in `rustbase-api`, 127/127 in `rustbase-db`. The `state_with_realm_and_admin` helper provisions a canonical `mobile` app so downstream tests have something to target.

### Dashboard

- ✅ `Users` and `OAuth providers` tabs live under `/realms/[realm]/apps/[app]/...` and use the new API URLs.
- ✅ Realm-level page nav strips drop Users/OAuth; app-level nav adds them.
- ✅ Production build (`bun --cwd ui run build`) succeeds.

### Docs

- ✅ `docs/concepts/mental-model.md` — users live per-app.
- ✅ `docs/concepts/storage-layout.md` — `users` table lives in `data.db`.
- ✅ `docs/guide/authentication.md` — URLs + master-admin username flow.
- ✅ `docs/guide/first-app.md` — curl examples threaded through the app segment.
- ✅ `docs/guide/introduction.md` + `docs/index.md` — realm/app description matches reality.
- ✅ `docs/guide/hooks.md` — clarifies that user-lifecycle hooks are app-scoped.
- ✅ `docs/reference/rest-api.md` — full URL refresh.

## Verification commands

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
bun --cwd ui run build
```

## Rollback

If you change your mind and want to drop the branch:

```sh
git checkout main
git branch -D feature/users-per-app
```
