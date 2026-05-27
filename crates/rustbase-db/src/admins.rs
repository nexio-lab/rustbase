//! Storage of the three admin kinds.
//!
//! - `MasterAdmin` rows live in `system.db`.
//! - `RealmAdmin` and `AppAdmin` rows live in their realm's `realm.db`.
//!
//! The structs hold no password material beyond the PHC-encoded hash;
//! plain-text passwords are only handled by `rustbase-auth`.

use crate::error::{DbError, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct MasterAdmin {
    pub id: String,
    pub username: String,
    pub email: Option<String>,
    pub password_hash: Option<String>,
    pub name: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct RealmAdmin {
    pub id: String,
    pub email: String,
    pub password_hash: String,
    pub name: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct AppAdmin {
    pub id: String,
    pub app_id: String,
    pub email: String,
    pub password_hash: String,
    pub name: Option<String>,
    pub created_at: DateTime<Utc>,
}

// ---- master admins (system.db) ---------------------------------------------

const SELECT_MASTER: &str =
    "SELECT id, username, email, password_hash, name, created_at FROM master_admins";

/// Insert the default `admin` master admin row on first boot if it
/// doesn't exist yet. `password_hash` is left NULL; the setup wizard
/// promotes the row to "initialized" by writing a real hash.
pub async fn ensure_seed_master_admin(pool: &SqlitePool) -> Result<()> {
    let n = count_master_admins(pool).await?;
    if n > 0 {
        return Ok(());
    }
    let id = Uuid::now_v7().to_string();
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO master_admins (id, username, email, password_hash, name, created_at) \
         VALUES (?, ?, NULL, NULL, ?, ?)",
    )
    .bind(&id)
    .bind("admin")
    .bind("admin")
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

/// Test / migration helper. Inserts a fully-formed admin row. The
/// production path uses `ensure_seed_master_admin` + `set_master_admin_password`.
pub async fn insert_master_admin(
    pool: &SqlitePool,
    username: &str,
    password_hash: &str,
    name: Option<&str>,
) -> Result<MasterAdmin> {
    let id = Uuid::now_v7().to_string();
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO master_admins (id, username, email, password_hash, name, created_at) \
         VALUES (?, ?, NULL, ?, ?, ?)",
    )
    .bind(&id)
    .bind(username)
    .bind(password_hash)
    .bind(name)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(MasterAdmin {
        id,
        username: username.to_string(),
        email: None,
        password_hash: Some(password_hash.to_string()),
        name: name.map(str::to_string),
        created_at: now,
    })
}

pub async fn find_master_admin_by_username(
    pool: &SqlitePool,
    username: &str,
) -> Result<Option<MasterAdmin>> {
    let sql = format!("{SELECT_MASTER} WHERE username = ?");
    let row: Option<MasterAdmin> = sqlx::query_as(&sql)
        .bind(username)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

pub async fn find_master_admin_by_id(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<MasterAdmin>> {
    let sql = format!("{SELECT_MASTER} WHERE id = ?");
    let row: Option<MasterAdmin> = sqlx::query_as(&sql).bind(id).fetch_optional(pool).await?;
    Ok(row)
}

/// Setup-time helper. Sets the password hash on an existing admin row.
/// Errors with RowNotFound if no such id.
pub async fn set_master_admin_password(
    pool: &SqlitePool,
    id: &str,
    password_hash: &str,
) -> Result<()> {
    let res = sqlx::query("UPDATE master_admins SET password_hash = ? WHERE id = ?")
        .bind(password_hash)
        .bind(id)
        .execute(pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::Sqlx(sqlx::Error::RowNotFound));
    }
    Ok(())
}

/// Server is "initialized" once at least one master admin has a
/// non-NULL password_hash — i.e. the setup wizard has been completed.
pub async fn master_admin_is_initialized(pool: &SqlitePool) -> Result<bool> {
    let n: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM master_admins WHERE password_hash IS NOT NULL")
            .fetch_one(pool)
            .await?;
    Ok(n > 0)
}

pub async fn count_master_admins(pool: &SqlitePool) -> Result<i64> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM master_admins")
        .fetch_one(pool)
        .await?;
    Ok(n)
}

pub async fn delete_master_admin(pool: &SqlitePool, id: &str) -> Result<()> {
    let res = sqlx::query("DELETE FROM master_admins WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::Sqlx(sqlx::Error::RowNotFound));
    }
    Ok(())
}

// ---- realm admins (realm.db) -----------------------------------------------

pub async fn insert_realm_admin(
    pool: &SqlitePool,
    email: &str,
    password_hash: &str,
    name: Option<&str>,
) -> Result<RealmAdmin> {
    let id = Uuid::now_v7().to_string();
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO realm_admins (id, email, password_hash, name, created_at) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(email)
    .bind(password_hash)
    .bind(name)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(RealmAdmin {
        id,
        email: email.to_string(),
        password_hash: password_hash.to_string(),
        name: name.map(str::to_string),
        created_at: now,
    })
}

pub async fn find_realm_admin_by_email(
    pool: &SqlitePool,
    email: &str,
) -> Result<Option<RealmAdmin>> {
    let row: Option<RealmAdmin> = sqlx::query_as(
        "SELECT id, email, password_hash, name, created_at FROM realm_admins WHERE email = ?",
    )
    .bind(email)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

// ---- app admins (realm.db, scoped to a single app) -------------------------

pub async fn insert_app_admin(
    pool: &SqlitePool,
    app_id: &str,
    email: &str,
    password_hash: &str,
    name: Option<&str>,
) -> Result<AppAdmin> {
    let id = Uuid::now_v7().to_string();
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO app_admins (id, app_id, email, password_hash, name, created_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(app_id)
    .bind(email)
    .bind(password_hash)
    .bind(name)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(AppAdmin {
        id,
        app_id: app_id.to_string(),
        email: email.to_string(),
        password_hash: password_hash.to_string(),
        name: name.map(str::to_string),
        created_at: now,
    })
}

pub async fn find_app_admin_by_email(
    pool: &SqlitePool,
    app_id: &str,
    email: &str,
) -> Result<Option<AppAdmin>> {
    let row: Option<AppAdmin> = sqlx::query_as(
        "SELECT id, app_id, email, password_hash, name, created_at FROM app_admins \
         WHERE app_id = ? AND email = ?",
    )
    .bind(app_id)
    .bind(email)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::{REALM_MIGRATIONS, SYSTEM_MIGRATIONS, apply_migrations};
    use crate::pool::open_memory_pool;

    async fn system_pool() -> SqlitePool {
        let pool = open_memory_pool().await.unwrap();
        apply_migrations(pool.clone(), SYSTEM_MIGRATIONS)
            .await
            .unwrap();
        pool
    }

    async fn realm_pool() -> SqlitePool {
        let pool = open_memory_pool().await.unwrap();
        apply_migrations(pool.clone(), REALM_MIGRATIONS)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn master_admin_insert_then_find() {
        let pool = system_pool().await;
        let inserted = insert_master_admin(&pool, "admin", "$argon2id$..hash..", Some("Ada"))
            .await
            .unwrap();
        let found = find_master_admin_by_username(&pool, "admin")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found, inserted);
        assert_eq!(count_master_admins(&pool).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn master_admin_username_is_unique() {
        let pool = system_pool().await;
        insert_master_admin(&pool, "admin", "h", None)
            .await
            .unwrap();
        let err = insert_master_admin(&pool, "admin", "h2", None)
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::Sqlx(_)));
    }

    #[tokio::test]
    async fn seed_master_admin_is_idempotent_and_uninitialized() {
        let pool = system_pool().await;
        ensure_seed_master_admin(&pool).await.unwrap();
        ensure_seed_master_admin(&pool).await.unwrap();
        assert_eq!(count_master_admins(&pool).await.unwrap(), 1);
        assert!(!master_admin_is_initialized(&pool).await.unwrap());

        // Look up the seeded row, set its password, and confirm
        // initialization flips.
        let seed = find_master_admin_by_username(&pool, "admin")
            .await
            .unwrap()
            .unwrap();
        assert!(seed.password_hash.is_none());
        set_master_admin_password(&pool, &seed.id, "$argon2id$..")
            .await
            .unwrap();
        assert!(master_admin_is_initialized(&pool).await.unwrap());
    }

    #[tokio::test]
    async fn delete_master_admin_reports_missing() {
        let pool = system_pool().await;
        let err = delete_master_admin(&pool, "does-not-exist")
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::Sqlx(sqlx::Error::RowNotFound)));
    }

    #[tokio::test]
    async fn realm_admin_insert_then_find() {
        let pool = realm_pool().await;
        let inserted = insert_realm_admin(&pool, "ops@acme.com", "hash", Some("Ops"))
            .await
            .unwrap();
        let found = find_realm_admin_by_email(&pool, "ops@acme.com")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found, inserted);
    }

    #[tokio::test]
    async fn app_admin_scoped_by_app_id() {
        let pool = realm_pool().await;
        // Need an `apps` row because app_admins.app_id has a FK.
        sqlx::query("INSERT INTO apps (id, name, created_at) VALUES (?, ?, ?)")
            .bind("mobile")
            .bind("Mobile")
            .bind(Utc::now())
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO apps (id, name, created_at) VALUES (?, ?, ?)")
            .bind("web")
            .bind("Web")
            .bind(Utc::now())
            .execute(&pool)
            .await
            .unwrap();

        insert_app_admin(&pool, "mobile", "a@x.com", "h", None)
            .await
            .unwrap();
        insert_app_admin(&pool, "web", "a@x.com", "h", None)
            .await
            .unwrap();

        let mobile_admin = find_app_admin_by_email(&pool, "mobile", "a@x.com")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(mobile_admin.app_id, "mobile");

        let no_such = find_app_admin_by_email(&pool, "admin", "a@x.com")
            .await
            .unwrap();
        assert!(no_such.is_none());
    }
}
