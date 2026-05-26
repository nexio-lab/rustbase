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
    pub email: String,
    pub password_hash: String,
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

pub async fn insert_master_admin(
    pool: &SqlitePool,
    email: &str,
    password_hash: &str,
    name: Option<&str>,
) -> Result<MasterAdmin> {
    let id = Uuid::now_v7().to_string();
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO master_admins (id, email, password_hash, name, created_at) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(email)
    .bind(password_hash)
    .bind(name)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(MasterAdmin {
        id,
        email: email.to_string(),
        password_hash: password_hash.to_string(),
        name: name.map(str::to_string),
        created_at: now,
    })
}

pub async fn find_master_admin_by_email(
    pool: &SqlitePool,
    email: &str,
) -> Result<Option<MasterAdmin>> {
    let row: Option<MasterAdmin> = sqlx::query_as(
        "SELECT id, email, password_hash, name, created_at FROM master_admins WHERE email = ?",
    )
    .bind(email)
    .fetch_optional(pool)
    .await?;
    Ok(row)
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
        let inserted =
            insert_master_admin(&pool, "ada@example.com", "$argon2id$..hash..", Some("Ada"))
                .await
                .unwrap();
        let found = find_master_admin_by_email(&pool, "ada@example.com")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found, inserted);
        assert_eq!(count_master_admins(&pool).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn master_admin_email_is_unique() {
        let pool = system_pool().await;
        insert_master_admin(&pool, "a@example.com", "h", None)
            .await
            .unwrap();
        let err = insert_master_admin(&pool, "a@example.com", "h2", None)
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::Sqlx(_)));
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
