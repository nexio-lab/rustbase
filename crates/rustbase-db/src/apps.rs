//! App rows inside a realm's `realm.db`.

use crate::error::{DbError, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct App {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

pub async fn create_app(pool: &SqlitePool, id: &str, name: &str) -> Result<App> {
    let now = Utc::now();
    sqlx::query("INSERT INTO apps (id, name, created_at) VALUES (?, ?, ?)")
        .bind(id)
        .bind(name)
        .bind(now)
        .execute(pool)
        .await?;
    Ok(App {
        id: id.to_string(),
        name: name.to_string(),
        created_at: now,
    })
}

pub async fn find_app(pool: &SqlitePool, id: &str) -> Result<Option<App>> {
    let row: Option<App> = sqlx::query_as("SELECT id, name, created_at FROM apps WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

pub async fn list_apps(pool: &SqlitePool) -> Result<Vec<App>> {
    let rows: Vec<App> =
        sqlx::query_as("SELECT id, name, created_at FROM apps ORDER BY created_at ASC")
            .fetch_all(pool)
            .await?;
    Ok(rows)
}

pub async fn rename_app(pool: &SqlitePool, id: &str, new_name: &str) -> Result<()> {
    let res = sqlx::query("UPDATE apps SET name = ? WHERE id = ?")
        .bind(new_name)
        .bind(id)
        .execute(pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::Sqlx(sqlx::Error::RowNotFound));
    }
    Ok(())
}

pub async fn delete_app(pool: &SqlitePool, id: &str) -> Result<()> {
    let res = sqlx::query("DELETE FROM apps WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::Sqlx(sqlx::Error::RowNotFound));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::{REALM_MIGRATIONS, apply_migrations};
    use crate::pool::open_memory_pool;

    async fn fresh_pool() -> SqlitePool {
        let pool = open_memory_pool().await.unwrap();
        apply_migrations(pool.clone(), REALM_MIGRATIONS)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn create_then_find_app() {
        let pool = fresh_pool().await;
        let inserted = create_app(&pool, "mobile", "Mobile").await.unwrap();
        let found = find_app(&pool, "mobile").await.unwrap().unwrap();
        assert_eq!(inserted, found);
    }

    #[tokio::test]
    async fn list_apps_orders_by_created_at() {
        let pool = fresh_pool().await;
        create_app(&pool, "a", "A").await.unwrap();
        create_app(&pool, "b", "B").await.unwrap();
        let apps = list_apps(&pool).await.unwrap();
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].id, "a");
        assert_eq!(apps[1].id, "b");
    }

    #[tokio::test]
    async fn rename_app_changes_name() {
        let pool = fresh_pool().await;
        create_app(&pool, "x", "First").await.unwrap();
        rename_app(&pool, "x", "Renamed").await.unwrap();
        let app = find_app(&pool, "x").await.unwrap().unwrap();
        assert_eq!(app.name, "Renamed");
    }

    #[tokio::test]
    async fn rename_unknown_returns_row_not_found() {
        let pool = fresh_pool().await;
        let err = rename_app(&pool, "absent", "x").await.unwrap_err();
        assert!(matches!(err, DbError::Sqlx(sqlx::Error::RowNotFound)));
    }

    #[tokio::test]
    async fn delete_app_removes_row_and_cascades_admins() {
        let pool = fresh_pool().await;
        create_app(&pool, "y", "Y").await.unwrap();
        crate::admins::insert_app_admin(&pool, "y", "a@b.c", "h", None)
            .await
            .unwrap();
        let admin_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM app_admins WHERE app_id = ?")
                .bind("y")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(admin_count, 1);

        delete_app(&pool, "y").await.unwrap();
        let app_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM apps")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(app_count, 0);
        // FK ON DELETE CASCADE drops the matching app_admins row too
        let admin_after: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM app_admins WHERE app_id = ?")
                .bind("y")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(admin_after, 0);
    }
}
