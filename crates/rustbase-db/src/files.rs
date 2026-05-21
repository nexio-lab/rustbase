//! Metadata for files uploaded to an app's storage.
//!
//! Binary bytes live in the `object_store` backend; this table only
//! tracks the `(id, filename, mime, size, created_at)` row that
//! everything else (links from records, ACL, listings) keys against.

use crate::error::{DbError, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct FileMeta {
    pub id: String,
    pub filename: String,
    pub mime: Option<String>,
    pub size: i64,
    pub created_at: DateTime<Utc>,
}

pub async fn insert_file(
    pool: &SqlitePool,
    filename: &str,
    mime: Option<&str>,
    size: i64,
) -> Result<FileMeta> {
    let id = Uuid::now_v7().to_string();
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO _files (id, filename, mime, size, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(filename)
    .bind(mime)
    .bind(size)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(FileMeta {
        id,
        filename: filename.to_string(),
        mime: mime.map(str::to_string),
        size,
        created_at: now,
    })
}

pub async fn find_file(pool: &SqlitePool, id: &str) -> Result<Option<FileMeta>> {
    let row: Option<FileMeta> = sqlx::query_as(
        "SELECT id, filename, mime, size, created_at FROM _files WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn list_files(pool: &SqlitePool) -> Result<Vec<FileMeta>> {
    let rows: Vec<FileMeta> = sqlx::query_as(
        "SELECT id, filename, mime, size, created_at FROM _files \
         ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn delete_file(pool: &SqlitePool, id: &str) -> Result<()> {
    let res = sqlx::query("DELETE FROM _files WHERE id = ?")
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
    use crate::migrations::{APP_MIGRATIONS, apply_migrations};
    use crate::pool::open_memory_pool;

    async fn fresh_pool() -> SqlitePool {
        let pool = open_memory_pool().await.unwrap();
        apply_migrations(pool.clone(), APP_MIGRATIONS).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn insert_then_find_round_trip() {
        let pool = fresh_pool().await;
        let m = insert_file(&pool, "kitten.png", Some("image/png"), 4096)
            .await
            .unwrap();
        let got = find_file(&pool, &m.id).await.unwrap().unwrap();
        assert_eq!(got.filename, "kitten.png");
        assert_eq!(got.mime.as_deref(), Some("image/png"));
        assert_eq!(got.size, 4096);
    }

    #[tokio::test]
    async fn list_orders_newest_first() {
        let pool = fresh_pool().await;
        insert_file(&pool, "a.txt", None, 1).await.unwrap();
        // ensure differing created_at
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        insert_file(&pool, "b.txt", None, 1).await.unwrap();
        let listed = list_files(&pool).await.unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].filename, "b.txt");
    }

    #[tokio::test]
    async fn delete_unknown_returns_row_not_found() {
        let pool = fresh_pool().await;
        let err = delete_file(&pool, "ghost").await.unwrap_err();
        assert!(matches!(err, DbError::Sqlx(sqlx::Error::RowNotFound)));
    }
}
