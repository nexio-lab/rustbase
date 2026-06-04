//! Scoped migration runner.
//!
//! A `Migration` is owned by exactly one `MigrationScope` (system /
//! workspace / app) and carries multi-statement SQL. The runner
//! records applied migrations in a per-pool `_migrations` table,
//! skipping any already present, and runs each new migration inside a
//! transaction. If the SQL fails the transaction is rolled back and
//! the migration is reported as pending on next boot.

use crate::error::{DbError, Result};
use chrono::Utc;
use sqlx::SqlitePool;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationScope {
    System,
    Workspace,
    App,
}

#[derive(Debug, Clone)]
pub struct Migration {
    pub id: &'static str,
    pub scope: MigrationScope,
    pub sql: &'static str,
}

impl Migration {
    pub const fn new(id: &'static str, scope: MigrationScope, sql: &'static str) -> Self {
        Self { id, scope, sql }
    }
}

/// Bookkeeping table that the runner installs the first time it runs
/// against a pool. Held as a `const` so the source is greppable.
const ENSURE_MIGRATIONS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS _migrations (
    id TEXT PRIMARY KEY,
    applied_at TEXT NOT NULL,
    duration_ms INTEGER NOT NULL
);
"#;

/// Apply every migration in `migrations` whose `id` is not yet recorded
/// in the pool's `_migrations` table, in the order supplied. Each
/// migration runs in its own transaction.
///
/// `pool` is taken by clone (SqlitePool is `Arc`-backed) so the
/// returned future has no captured borrow, sidestepping a known sqlx
/// HRTB inference quirk that otherwise prevents this future from being
/// `Send` when composed inside an axum handler.
pub async fn apply_migrations(pool: SqlitePool, migrations: &[Migration]) -> Result<usize> {
    sqlx::raw_sql(ENSURE_MIGRATIONS_TABLE)
        .execute(&pool)
        .await?;

    let already: HashSet<String> = sqlx::query_scalar::<_, String>("SELECT id FROM _migrations")
        .fetch_all(&pool)
        .await?
        .into_iter()
        .collect();

    let mut applied = 0usize;
    for m in migrations {
        if already.contains(m.id) {
            continue;
        }
        let elapsed = apply_one(pool.clone(), m.clone()).await?;
        tracing::info!(migration = m.id, elapsed_ms = elapsed, "applied migration");
        applied += 1;
    }
    Ok(applied)
}

/// Apply one migration atomically.
///
/// Atomicity for the migration SQL is supplied by wrapping the script
/// in `BEGIN; ... COMMIT;` and executing it as one `raw_sql` call on
/// the pool — SQLite parses this as a single explicit transaction. If
/// any statement fails, the BEGIN's transaction is implicitly rolled
/// back when the connection is returned to the pool. The
/// `_migrations` row is then inserted in a second statement; if that
/// fails (e.g., transient I/O), the migration stays pending and
/// retries on the next boot — DDL must therefore remain idempotent
/// under retry (use `IF NOT EXISTS` for any future schema).
///
/// This shape sidesteps a known sqlx HRTB inference quirk that breaks
/// `Send` recognition for futures that hold a `&mut Transaction`
/// across an `.await`, which otherwise prevents this code from
/// composing into an axum handler.
async fn apply_one(pool: SqlitePool, m: Migration) -> Result<i64> {
    let start = std::time::Instant::now();

    let wrapped = format!("BEGIN;\n{}\nCOMMIT;", m.sql);
    if let Err(source) = sqlx::raw_sql(&wrapped).execute(&pool).await {
        // Best-effort rollback so any partially-applied DDL inside the
        // open transaction is reverted before the next caller observes
        // the connection. ROLLBACK with no active transaction is a
        // benign error and is ignored.
        let _ = sqlx::raw_sql("ROLLBACK").execute(&pool).await;
        return Err(DbError::Migration {
            migration: m.id.to_string(),
            source,
        });
    }

    let elapsed = i64::try_from(start.elapsed().as_millis()).unwrap_or(i64::MAX);
    sqlx::query("INSERT INTO _migrations (id, applied_at, duration_ms) VALUES (?, ?, ?)")
        .bind(m.id)
        .bind(Utc::now().to_rfc3339())
        .bind(elapsed)
        .execute(&pool)
        .await?;

    Ok(elapsed)
}

// -----------------------------------------------------------------------------
// Initial schemas. Future schema changes get new migration entries with newer
// timestamp IDs; existing entries are never edited.
// -----------------------------------------------------------------------------

pub const SYSTEM_MIGRATIONS: &[Migration] = &[
    Migration::new(
        "20260520_000001_initial_system",
        MigrationScope::System,
        r#"
        CREATE TABLE workspaces (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            is_master INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL
        );
        CREATE UNIQUE INDEX workspaces_one_master ON workspaces(is_master) WHERE is_master = 1;

        -- Master admin identity. On first boot, exactly one row is
        -- auto-seeded with username='admin' and password_hash=NULL —
        -- the setup wizard then sets the password. Email is optional;
        -- only the username is the login identity.
        CREATE TABLE master_admins (
            id TEXT PRIMARY KEY,
            username TEXT NOT NULL UNIQUE,
            email TEXT,
            password_hash TEXT,
            name TEXT,
            created_at TEXT NOT NULL
        );

        CREATE TABLE policies (
            field TEXT PRIMARY KEY,
            policy_json TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE audit_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            ts TEXT NOT NULL,
            actor TEXT,
            action TEXT NOT NULL,
            target TEXT,
            details_json TEXT
        );
        CREATE INDEX audit_log_ts ON audit_log(ts);
        "#,
    ),
    Migration::new(
        "20260521_000001_master_auth_storage",
        MigrationScope::System,
        r#"
        CREATE TABLE _secrets (
            name TEXT PRIMARY KEY,
            value BLOB NOT NULL,
            created_at TEXT NOT NULL
        );

        CREATE TABLE _refresh_tokens (
            token TEXT PRIMARY KEY,
            subject_kind TEXT NOT NULL,
            subject_id TEXT NOT NULL,
            issued_at TEXT NOT NULL,
            expires_at TEXT NOT NULL,
            revoked INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX master_refresh_tokens_subject
            ON _refresh_tokens(subject_kind, subject_id);
        "#,
    ),
];

pub const WORKSPACE_MIGRATIONS: &[Migration] = &[
    Migration::new(
        "20260520_000001_initial_workspace",
        MigrationScope::Workspace,
        r#"
    CREATE TABLE apps (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        created_at TEXT NOT NULL
    );

    CREATE TABLE workspace_admins (
        id TEXT PRIMARY KEY,
        email TEXT NOT NULL UNIQUE,
        password_hash TEXT NOT NULL,
        name TEXT,
        created_at TEXT NOT NULL
    );

    CREATE TABLE app_admins (
        id TEXT PRIMARY KEY,
        app_id TEXT NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
        email TEXT NOT NULL,
        password_hash TEXT NOT NULL,
        name TEXT,
        created_at TEXT NOT NULL,
        UNIQUE(app_id, email)
    );

    -- Refresh tokens for workspace-scope subjects: workspace_admin /
    -- app_admin. End-user refresh tokens still live in the per-app
    -- data.db along with the per-app users table (workspace-shared
    -- identity lands in a later migration).
    CREATE TABLE _refresh_tokens (
        token TEXT PRIMARY KEY,
        subject_kind TEXT NOT NULL,
        subject_id TEXT NOT NULL,
        issued_at TEXT NOT NULL,
        expires_at TEXT NOT NULL,
        revoked INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX refresh_tokens_subject ON _refresh_tokens(subject_kind, subject_id);

    CREATE TABLE policies (
        field TEXT PRIMARY KEY,
        policy_json TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );

    CREATE TABLE audit_log (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        ts TEXT NOT NULL,
        actor TEXT,
        action TEXT NOT NULL,
        target TEXT,
        details_json TEXT
    );
    CREATE INDEX audit_log_ts ON audit_log(ts);
    "#,
    ),
    Migration::new(
        // End-user identity is workspace-scoped: one `users` row per email
        // per workspace, usable across every app in that workspace
        // (PocketBase / Supabase shape). OAuth providers, OTP, verify-email,
        // password-reset, TOTP, MFA challenges and OAuth state nonces all
        // sit alongside. End-user refresh tokens reuse the workspace-level
        // `_refresh_tokens` table (subject_kind discriminates).
        "20260606_000001_workspace_user_identity",
        MigrationScope::Workspace,
        r#"
    CREATE TABLE users (
        id TEXT PRIMARY KEY,
        email TEXT NOT NULL UNIQUE,
        password_hash TEXT,
        verified INTEGER NOT NULL DEFAULT 0,
        last_login TEXT,
        created_at TEXT NOT NULL
    );

    CREATE TABLE oauth_providers (
        provider TEXT PRIMARY KEY,
        client_id TEXT NOT NULL,
        client_secret_enc TEXT NOT NULL,
        config_json TEXT
    );

    CREATE TABLE user_oauth_links (
        user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        provider TEXT NOT NULL,
        provider_user_id TEXT NOT NULL,
        PRIMARY KEY (user_id, provider)
    );

    CREATE TABLE _email_verifications (
        token TEXT PRIMARY KEY,
        user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        issued_at TEXT NOT NULL,
        expires_at TEXT NOT NULL,
        consumed_at TEXT
    );
    CREATE INDEX email_verifications_user ON _email_verifications(user_id);

    CREATE TABLE _password_resets (
        token TEXT PRIMARY KEY,
        user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        issued_at TEXT NOT NULL,
        expires_at TEXT NOT NULL,
        consumed_at TEXT
    );
    CREATE INDEX password_resets_user ON _password_resets(user_id);

    -- One-time numeric codes for passwordless / 2FA email login.
    -- Keyed by email rather than user_id so OTP can double as a
    -- sign-up channel (the user row may not exist yet on first
    -- request). New requests for the same email invalidate prior
    -- unconsumed codes (single in-flight code per email).
    CREATE TABLE _email_otps (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        code TEXT NOT NULL,
        email TEXT NOT NULL,
        issued_at TEXT NOT NULL,
        expires_at TEXT NOT NULL,
        consumed_at TEXT,
        attempts INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX email_otps_lookup ON _email_otps(email, consumed_at);

    -- CSRF state nonces + PKCE verifier for the OAuth2 authorization
    -- code flow. Issued by /authorize, consumed at /callback.
    CREATE TABLE _oauth_states (
        state TEXT PRIMARY KEY,
        provider TEXT NOT NULL,
        redirect_uri TEXT NOT NULL,
        issued_at TEXT NOT NULL,
        expires_at TEXT NOT NULL,
        consumed_at TEXT,
        code_verifier TEXT
    );
    CREATE INDEX oauth_states_provider ON _oauth_states(provider, consumed_at);

    CREATE TABLE _user_totp (
        user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
        secret_b32 TEXT NOT NULL,
        enrolled_at TEXT NOT NULL,
        confirmed_at TEXT,
        enabled INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE _mfa_challenges (
        token TEXT PRIMARY KEY,
        user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        issued_at TEXT NOT NULL,
        expires_at TEXT NOT NULL,
        consumed_at TEXT
    );
    CREATE INDEX mfa_challenges_user ON _mfa_challenges(user_id, consumed_at);
    "#,
    ),
];

pub const APP_MIGRATIONS: &[Migration] = &[
    Migration::new(
        "20260520_000001_initial_app",
        MigrationScope::App,
        r#"
        CREATE TABLE _collections (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            kind TEXT NOT NULL,
            schema_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE _access_rules (
            collection_id TEXT NOT NULL REFERENCES _collections(id) ON DELETE CASCADE,
            action TEXT NOT NULL,
            filter TEXT,
            PRIMARY KEY (collection_id, action)
        );

        CREATE TABLE policies (
            field TEXT PRIMARY KEY,
            policy_json TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE audit_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            ts TEXT NOT NULL,
            actor TEXT,
            action TEXT NOT NULL,
            target TEXT,
            details_json TEXT
        );
        CREATE INDEX audit_log_ts ON audit_log(ts);
        "#,
    ),
    Migration::new(
        "20260521_000002_app_files",
        MigrationScope::App,
        r#"
        CREATE TABLE _files (
            id TEXT PRIMARY KEY,
            filename TEXT NOT NULL,
            mime TEXT,
            size INTEGER NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE INDEX files_created_at ON _files(created_at);
        "#,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::open_memory_pool;

    #[tokio::test]
    async fn applies_system_migrations_once_and_is_idempotent() {
        let pool = open_memory_pool().await.unwrap();
        let n1 = apply_migrations(pool.clone(), SYSTEM_MIGRATIONS)
            .await
            .unwrap();
        let n2 = apply_migrations(pool.clone(), SYSTEM_MIGRATIONS)
            .await
            .unwrap();
        assert_eq!(n1, SYSTEM_MIGRATIONS.len());
        assert_eq!(n2, 0);

        // verify a table from the system schema actually exists
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM workspaces")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn applies_workspace_migrations() {
        let pool = open_memory_pool().await.unwrap();
        let n = apply_migrations(pool.clone(), WORKSPACE_MIGRATIONS)
            .await
            .unwrap();
        assert_eq!(n, WORKSPACE_MIGRATIONS.len());

        // Workspace-scope tables: apps + admin tiers + workspace-shared
        // user identity (users, oauth providers, OTP, TOTP, MFA).
        for table in [
            "apps",
            "workspace_admins",
            "app_admins",
            "users",
            "oauth_providers",
            "user_oauth_links",
            "_email_verifications",
            "_password_resets",
            "_email_otps",
            "_oauth_states",
            "_user_totp",
            "_mfa_challenges",
        ] {
            let q = format!("SELECT COUNT(*) FROM {table}");
            let count: i64 = sqlx::query_scalar(&q).fetch_one(&pool).await.unwrap();
            assert_eq!(count, 0, "table {table} should be empty after migration");
        }
    }

    #[tokio::test]
    async fn applies_app_migrations() {
        let pool = open_memory_pool().await.unwrap();
        let n = apply_migrations(pool.clone(), APP_MIGRATIONS)
            .await
            .unwrap();
        assert_eq!(n, APP_MIGRATIONS.len());

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _collections")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn failing_migration_is_rolled_back() {
        let pool = open_memory_pool().await.unwrap();
        let bad = &[Migration::new(
            "20260520_000001_broken",
            MigrationScope::System,
            "CREATE TABLE good (x INTEGER); CREATE TABLE bad (this will not parse;",
        )];
        let err = apply_migrations(pool.clone(), bad).await.unwrap_err();
        assert!(matches!(err, DbError::Migration { .. }));

        // neither the good table nor a _migrations row should be present
        let row: Option<i64> =
            sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE name = 'good'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(row, Some(0));

        let applied: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _migrations")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(applied, 0);
    }

    #[tokio::test]
    async fn records_duration_in_migrations_table() {
        let pool = open_memory_pool().await.unwrap();
        apply_migrations(pool.clone(), SYSTEM_MIGRATIONS)
            .await
            .unwrap();
        let row: (String, i64) = sqlx::query_as("SELECT id, duration_ms FROM _migrations LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.0, SYSTEM_MIGRATIONS[0].id);
        assert!(row.1 >= 0);
    }
}
