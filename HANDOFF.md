# Handoff: feature/users-per-app

A WIP commit on `feature/users-per-app` captures partial work on a big two-part refactor. **The branch does not compile end-to-end yet.** This file documents what's done, what's left, and the design decisions so the next session can continue without re-deriving them.

## Two refactors, one branch

### Part A — master admin auto-seed (smaller)

**Goal:** On first boot, RustBaas auto-creates a master admin row with `username = "admin"` and `password_hash = NULL`. The setup wizard accepts only `{password}` and writes the hash. Master admin login uses `{username, password}`; the realm admins and end-users keep email-based login.

**Status:**

- ✅ `master_admins` schema updated: added `username TEXT NOT NULL UNIQUE`, made `email` and `password_hash` nullable. See [crates/rustbase-db/src/migrations.rs](crates/rustbase-db/src/migrations.rs).
- ✅ `MasterAdmin` struct + helpers rewritten in [crates/rustbase-db/src/admins.rs](crates/rustbase-db/src/admins.rs):
    - `ensure_seed_master_admin(pool)` — idempotent, inserts `admin` row with NULL password.
    - `find_master_admin_by_username` / `_by_id`
    - `set_master_admin_password(pool, id, hash)`
    - `master_admin_is_initialized(pool)` — true iff any admin has a non-NULL `password_hash`.
- ✅ `POST /_/setup` rewritten in [crates/rustbase-api/src/setup.rs](crates/rustbase-api/src/setup.rs) — takes `{password}`, looks up the `admin` row, updates its hash.
- ✅ `POST /_/auth/admin/login` rewritten in [crates/rustbase-api/src/auth/login.rs](crates/rustbase-api/src/auth/login.rs) — uses `MasterLoginRequest { username, password }`, returns `MasterLoginResponse` with `MasterAdminPublic { id, username, email: Option, name }`.

**Still to do:**

- ❌ `rustbase-server/src/main.rs` bootstrap must call `ensure_seed_master_admin` on first boot, **before** the setup-gate check. Right now the gate keys off `count_master_admins`; it should key off `master_admin_is_initialized` (so the seeded row doesn't unblock the gate).
- ❌ `crates/rustbase-api/src/middleware.rs` setup gate condition: switch from `count_master_admins() > 0` to `master_admin_is_initialized()`.
- ❌ Dashboard `ui/src/routes/setup/+page.svelte` collects only password. Currently asks for email + password + name.
- ❌ Dashboard `ui/src/routes/login/+page.svelte` field labels: master admin login is `username` (not `email`). Note the realm-admin login at `/realms/<realm>/auth/admin/login` keeps email — UI should render the right form based on context.
- ❌ Update `MasterLoginResponse` consumers in `ui/src/lib/api.ts` — the type currently has `admin.email: string`; change to `admin.username: string` plus `admin.email: string | null`.
- ❌ Refresh handler: master admin refresh tokens are in `system.db` already, no change needed for refresh. But the response shape needs the `MasterAdminPublic` (not `AdminPublic`). Check [crates/rustbase-api/src/auth/refresh.rs](crates/rustbase-api/src/auth/refresh.rs).

### Part B — users move from realm to app

**Goal:** End-users live in each app's `data.db`, not in the realm. Auth endpoints become `/api/realms/<realm>/apps/<app>/auth/users/...`. OAuth providers + every user-bound auxiliary table move with them. Realm admins keep cross-app reach (their token covers every app in the realm); app admins are scoped to one app.

**Status:**

- ✅ Migration tables moved in [crates/rustbase-db/src/migrations.rs](crates/rustbase-db/src/migrations.rs):
    - Moved from `REALM_MIGRATIONS` to `APP_MIGRATIONS`: `users`, `oauth_providers`, `user_oauth_links`, `_email_verifications`, `_password_resets`, `_email_otps`, `_oauth_states`, `_user_totp`, `_mfa_challenges`, and a per-app `_refresh_tokens`.
    - Realm keeps: `apps`, `realm_admins`, `app_admins`, `_refresh_tokens` (admin subjects only), `policies`, `audit_log`.

**Still to do (a lot):**

#### `rustbase-db` call-site cleanups

The DB helpers themselves are pool-agnostic (they take `&SqlitePool`), so no signature changes needed. But every test inside `rustbase-db` that bootstrapped users/oauth from `REALM_MIGRATIONS` now needs to apply `APP_MIGRATIONS` instead. Affected files include:

- `crates/rustbase-db/src/users.rs` (tests)
- `crates/rustbase-db/src/oauth_providers.rs` (tests)
- `crates/rustbase-db/src/oauth_links.rs` (tests)
- `crates/rustbase-db/src/oauth_states.rs` (tests)
- `crates/rustbase-db/src/email_otps.rs` (tests)
- `crates/rustbase-db/src/email_verifications.rs` (tests)
- `crates/rustbase-db/src/password_resets.rs` (tests)
- `crates/rustbase-db/src/user_totp.rs` (tests)
- `crates/rustbase-db/src/mfa_challenges.rs` (tests)
- `crates/rustbase-db/src/tokens.rs` (tests; the refresh-token suite needs split — some test cases write admin subjects against realm pool, some write user subjects against app pool)

Pattern: where a test currently does `apply_migrations(pool, REALM_MIGRATIONS)`, switch to `apply_migrations(pool, APP_MIGRATIONS)` and remove the `app_admins`/`realm_admins` test setup, since those tables aren't in app scope anymore.

#### `rustbase-api` handlers

Every end-user / OAuth endpoint takes `Path<String>` (realm) today. Pattern after refactor:

```rust
// Before
pub async fn user_login(
    State(state): State<AppState>,
    Path(realm): Path<String>,
    Json(req): Json<LoginRequest>,
) -> ...
{
    let realm_id = RealmId::from(realm.clone());
    let pool = state.realms.pool_for(&realm_id).await?;
    // ...
}

// After
pub async fn user_login(
    State(state): State<AppState>,
    Path((realm, app)): Path<(String, String)>,
    Json(req): Json<LoginRequest>,
) -> ...
{
    let realm_id = RealmId::from(realm.clone());
    let app_id = AppId::from(app.clone());
    // require_app_exists check (see crates/rustbase-api/src/hooks.rs for the helper shape)
    let pool = state.apps.pool_for(&realm_id, &app_id).await?;
    // build_claims passes Some(app.clone()) for the app slot
    // ...
}
```

Files needing this update:

- `crates/rustbase-api/src/auth/login.rs` — `realm_admin_login` stays as-is (realm-scoped), but `user_login` becomes app-scoped.
- `crates/rustbase-api/src/auth/register.rs`
- `crates/rustbase-api/src/auth/refresh.rs` — split: `master_admin_refresh` stays system.db, `realm_admin_refresh` stays realm.db, `user_refresh` moves to app.db.
- `crates/rustbase-api/src/auth/verify_email.rs`
- `crates/rustbase-api/src/auth/password_reset.rs`
- `crates/rustbase-api/src/auth/email_otp.rs`
- `crates/rustbase-api/src/auth/totp.rs`
- `crates/rustbase-api/src/auth/oauth.rs` — authorize + callback
- `crates/rustbase-api/src/auth/oauth_admin.rs` — list / get / put / delete
- `crates/rustbase-api/src/users.rs` — admin user mgmt (list / get / verify / totp-reset / delete)

#### `rustbase-runtime` hook dispatch

User-lifecycle hooks currently iterate every app in the realm. After refactor, fire only on the specific app:

- `HookEngine::dispatch_user_before_login(realm, ...)` → `dispatch_user_before_login(realm, app, ...)`
- Same for `dispatch_user_after_login`, `dispatch_user_after_register`.
- Remove the `apps_in_realm` helper if it has no other callers.

#### JWT claims

`TokenRole::User` JWTs currently carry `realm: Some(...)`, `app: None`. After refactor, they should carry `app: Some(...)` too — set in `build_claims` callsites.

`AdminAuth::require_realm_access` and `require_app_access` already handle the matrix correctly (master ⊇ realm ⊇ app), so no changes needed in [crates/rustbase-api/src/auth/extract.rs](crates/rustbase-api/src/auth/extract.rs).

#### Routes

In [crates/rustbase-api/src/router.rs](crates/rustbase-api/src/router.rs), every route currently at `/api/realms/{realm}/auth/...` and `/api/realms/{realm}/users/...` and `/api/realms/{realm}/auth/oauth/...` gains an `/apps/{app}/` segment.

Old:
```
/api/realms/{realm}/auth/users/register
/api/realms/{realm}/auth/users/login
/api/realms/{realm}/auth/users/refresh
/api/realms/{realm}/auth/verify-email/{request,confirm}
/api/realms/{realm}/auth/password-reset/{request,confirm}
/api/realms/{realm}/auth/otp/{request,login}
/api/realms/{realm}/auth/totp/{enroll,confirm,disable}
/api/realms/{realm}/auth/users/login/totp
/api/realms/{realm}/auth/oauth/{provider}/authorize
/api/realms/{realm}/auth/oauth/{provider}/callback
/api/realms/{realm}/auth/oauth/providers
/api/realms/{realm}/auth/oauth/providers/{provider}
/api/realms/{realm}/users
/api/realms/{realm}/users/{id}
/api/realms/{realm}/users/{id}/verify
/api/realms/{realm}/users/{id}/totp
```

New: prepend `/apps/{app}` to each path between `/realms/{realm}` and the next segment.

Realm-admin auth (`/api/realms/{realm}/auth/admin/login`, `/api/realms/{realm}/auth/refresh`) **stays** at realm scope.

#### Tests

`crates/rustbase-api/src/router.rs` test module has ~30 tests that hit `/api/realms/acme/auth/users/...` etc. All need to:

1. Bootstrap to a state that includes an app: `state_with_app_and_collection` already does this; reuse it as the base.
2. Re-path every URL.
3. Token issuance helpers — the user-token helper needs the new app claim.
4. `state_with_collection_and_user`, `state_with_realm_and_admin` helpers: probably need a sibling `state_with_app_and_user` that registers the user inside the app.

The OAuth admin tests in particular ([crates/rustbase-api/src/auth/oauth_admin.rs](crates/rustbase-api/src/auth/oauth_admin.rs)) need their `state_with_oauth_provider` helper updated.

#### Server bootstrap

[crates/rustbase-server/src/main.rs](crates/rustbase-server/src/main.rs) bootstrap order:

1. Open system pool, run system migrations.
2. **NEW:** `ensure_seed_master_admin(system_pool)` — Part A.
3. Ensure master realm exists.
4. **Initialization check** keys off `master_admin_is_initialized()` — the setup gate stays closed until the wizard runs.

#### Dashboard

- `ui/src/routes/setup/+page.svelte` — strip to password only.
- `ui/src/routes/login/+page.svelte` — master login becomes username/password. Realm-admin path (`/login/<realm>`?) stays email/password.
- `ui/src/lib/auth.svelte.ts` — `MasterAdmin` type loses required `email`, gains `username`.
- `ui/src/lib/api.ts` — `MasterLoginResponse` shape change. New `app_id` in user tokens.
- Move Users tab from `/realms/[realm]/users` to `/realms/[realm]/apps/[app]/users`.
- Move OAuth tab from `/realms/[realm]/oauth` to `/realms/[realm]/apps/[app]/oauth`.
- Update every nav tab strip on the realm-scoped pages to drop Users / OAuth.

#### Docs

The following docs pages contain claims that become **wrong** after Part B and need rewriting:

- `docs/concepts/mental-model.md` — "Realms hold the user pool" → "Apps hold the user pool"
- `docs/guide/authentication.md` — every example URL needs the new app segment; master login becomes username
- `docs/guide/getting-started.md` — first-app flow stays mostly correct
- `docs/guide/first-app.md` — re-thread the curl examples with the app segment
- `docs/guide/configuration.md` — no change
- `docs/reference/rest-api.md` — replace the User auth section + Admin user mgmt section + OAuth section with the new paths
- `docs/concepts/storage-layout.md` — "Users live in `<realm>/realm.db`" → "Users live in `<realm>/apps/<app>/data.db`"

## Mechanical patterns the next session should use

1. **Compile-driven refactor.** Run `cargo check -p rustbase-api 2>&1 | grep -E "^error|--> "` after every chunk. The compiler is the ground truth for which call sites still need updating.

2. **Bulk path replace.** For routes and tests, an `sd` / `sed` regex like `s|/api/realms/{realm}/auth/users/|/api/realms/{realm}/apps/{app}/auth/users/|g` gets most of the work done, then hand-fix the handler signatures.

3. **Keep admin paths alone.** `/_/auth/admin/login`, `/_/auth/refresh`, `/_/setup`, `/api/realms/{realm}/auth/admin/login`, `/api/realms/{realm}/auth/refresh`, `/api/realms/{realm}/admins` all stay where they are — they're admin-tier, not end-user-tier.

4. **Tests bootstrap order.** Most tests already do realm → app — they need `state_with_app_and_collection` or its sibling `state_with_app`. The end-user setup helper `state_with_collection_and_user` is the one that needs the most rework.

5. **Hooks.** `dispatch_user_*` becomes `(realm, app)`. The `HookRequest::system(realm, "", "_user")` call sites need the actual app id (it's already in scope by then).

## Reset path

If this refactor turns out to be more than you want to ship right now, the rollback is:

```sh
git checkout main
git branch -D feature/users-per-app
```

Nothing on `main` is broken; the dashboard and docs ship without this work and stand on their own.
