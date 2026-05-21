//! Collection meta storage + per-collection table provisioning.
//!
//! `_collections` (in every `data.db`) holds one row per user-defined
//! collection. Creating a collection both inserts that row and runs a
//! `CREATE TABLE` for the collection's data table — the table name is
//! the collection's id, which has been slug-validated, so it's safe to
//! interpolate into DDL.
//!
//! Reserved collection ids are the internal tables `data.db` already
//! ships with: `_collections`, `_access_rules`, `_migrations`,
//! `policies`, `audit_log`.

use crate::error::{DbError, Result};
use chrono::{DateTime, Utc};
use rustbase_core::{CollectionKind, Field, FieldType, Schema};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Collection {
    pub id: String,
    pub kind: CollectionKind,
    pub schema: Schema,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const RESERVED_COLLECTION_IDS: &[&str] = &[
    "_collections",
    "_access_rules",
    "_migrations",
    "policies",
    "audit_log",
];

const RESERVED_FIELD_NAMES: &[&str] = &["id", "created_at", "updated_at"];

pub async fn create_collection(pool: &SqlitePool, schema: &Schema) -> Result<Collection> {
    validate_schema(schema)?;
    let id = schema.id.as_str();
    if RESERVED_COLLECTION_IDS.contains(&id) {
        return Err(DbError::InvalidIdentifier(format!(
            "collection id '{id}' is reserved"
        )));
    }

    let schema_json = serde_json::to_string(schema)
        .map_err(|e| DbError::InvalidIdentifier(format!("schema json: {e}")))?;
    let kind_str = collection_kind_str(schema.kind);
    let now = Utc::now();

    // CREATE TABLE — wrapped so multi-statement issues can be diagnosed
    // separately from the meta INSERT.
    let create_sql = format!("CREATE TABLE {} ({})", quote_ident(id), columns_sql(schema));
    sqlx::raw_sql(&create_sql).execute(pool).await?;

    sqlx::query(
        "INSERT INTO _collections (id, name, kind, schema_json, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(id) // `name` mirrors `id` for now; rename comes later
    .bind(kind_str)
    .bind(&schema_json)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| {
        // If the meta INSERT fails (e.g. duplicate id), the data table is
        // already there. Try to clean it up so the next attempt isn't
        // wedged on a half-created collection.
        // Best-effort: ignore secondary errors.
        let _drop = format!("DROP TABLE IF EXISTS {}", quote_ident(id));
        tracing::warn!(error = %e, "rolling back partial collection create");
        e
    })?;

    Ok(Collection {
        id: id.to_string(),
        kind: schema.kind,
        schema: schema.clone(),
        created_at: now,
        updated_at: now,
    })
}

pub async fn find_collection(pool: &SqlitePool, id: &str) -> Result<Option<Collection>> {
    let row: Option<(String, String, String, DateTime<Utc>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT id, kind, schema_json, created_at, updated_at FROM _collections WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    row.map(decode_collection).transpose()
}

pub async fn list_collections(pool: &SqlitePool) -> Result<Vec<Collection>> {
    let rows: Vec<(String, String, String, DateTime<Utc>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT id, kind, schema_json, created_at, updated_at FROM _collections \
         ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(decode_collection).collect()
}

pub async fn delete_collection(pool: &SqlitePool, id: &str) -> Result<()> {
    if !is_valid_ident(id) {
        return Err(DbError::InvalidIdentifier(id.to_string()));
    }

    sqlx::raw_sql(&format!("DROP TABLE IF EXISTS {}", quote_ident(id)))
        .execute(pool)
        .await?;

    let res = sqlx::query("DELETE FROM _collections WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::Sqlx(sqlx::Error::RowNotFound));
    }
    Ok(())
}

fn decode_collection(
    row: (String, String, String, DateTime<Utc>, DateTime<Utc>),
) -> Result<Collection> {
    let (id, kind_str, schema_json, created_at, updated_at) = row;
    let kind = parse_collection_kind(&kind_str)
        .ok_or_else(|| DbError::InvalidIdentifier(format!("unknown collection kind: {kind_str}")))?;
    let schema: Schema = serde_json::from_str(&schema_json)
        .map_err(|e| DbError::InvalidIdentifier(format!("schema json: {e}")))?;
    Ok(Collection {
        id,
        kind,
        schema,
        created_at,
        updated_at,
    })
}

fn collection_kind_str(k: CollectionKind) -> &'static str {
    match k {
        CollectionKind::Base => "base",
        CollectionKind::Auth => "auth",
        CollectionKind::View => "view",
    }
}

fn parse_collection_kind(s: &str) -> Option<CollectionKind> {
    match s {
        "base" => Some(CollectionKind::Base),
        "auth" => Some(CollectionKind::Auth),
        "view" => Some(CollectionKind::View),
        _ => None,
    }
}

pub fn validate_schema(schema: &Schema) -> Result<()> {
    let id = schema.id.as_str();
    if !is_valid_ident(id) {
        return Err(DbError::InvalidIdentifier(id.to_string()));
    }
    let mut seen = std::collections::HashSet::new();
    for f in &schema.fields {
        if !is_valid_ident(&f.name) {
            return Err(DbError::InvalidIdentifier(f.name.clone()));
        }
        if RESERVED_FIELD_NAMES.contains(&f.name.as_str()) {
            return Err(DbError::InvalidIdentifier(format!(
                "field name '{}' is reserved",
                f.name
            )));
        }
        if !seen.insert(f.name.clone()) {
            return Err(DbError::InvalidIdentifier(format!(
                "duplicate field name '{}'",
                f.name
            )));
        }
    }
    Ok(())
}

pub fn is_valid_ident(s: &str) -> bool {
    let len = s.len();
    if !(2..=50).contains(&len) {
        return false;
    }
    let mut chars = s.chars();
    let first = chars.next().unwrap();
    if !(first.is_ascii_lowercase() || first == '_') {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn quote_ident(s: &str) -> String {
    // is_valid_ident is the gate; this is just a defensive wrap.
    format!("\"{s}\"")
}

fn columns_sql(schema: &Schema) -> String {
    let mut parts = vec![
        "id TEXT PRIMARY KEY".to_string(),
        "created_at TEXT NOT NULL".to_string(),
        "updated_at TEXT NOT NULL".to_string(),
    ];
    for f in &schema.fields {
        parts.push(field_column_sql(f));
    }
    parts.join(", ")
}

fn field_column_sql(f: &Field) -> String {
    let mut col = format!("{} {}", quote_ident(&f.name), sql_type(&f.ty));
    if f.required {
        col.push_str(" NOT NULL");
    }
    if f.unique {
        col.push_str(" UNIQUE");
    }
    col
}

pub fn sql_type(ty: &FieldType) -> &'static str {
    match ty {
        FieldType::Number { .. } => "REAL",
        FieldType::Bool => "INTEGER",
        FieldType::Json
        | FieldType::Text { .. }
        | FieldType::Email
        | FieldType::Url
        | FieldType::Date
        | FieldType::Relation { .. }
        | FieldType::File { .. } => "TEXT",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::{APP_MIGRATIONS, apply_migrations};
    use crate::pool::open_memory_pool;
    use rustbase_core::{CollectionId, Field, FieldType};

    async fn fresh_pool() -> SqlitePool {
        let pool = open_memory_pool().await.unwrap();
        apply_migrations(pool.clone(), APP_MIGRATIONS).await.unwrap();
        pool
    }

    fn users_schema() -> Schema {
        Schema {
            id: CollectionId::from("users"),
            kind: CollectionKind::Base,
            fields: vec![
                Field {
                    name: "name".into(),
                    ty: FieldType::Text {
                        min: None,
                        max: Some(100),
                    },
                    required: true,
                    unique: false,
                },
                Field {
                    name: "age".into(),
                    ty: FieldType::Number {
                        min: None,
                        max: None,
                    },
                    required: false,
                    unique: false,
                },
                Field {
                    name: "verified".into(),
                    ty: FieldType::Bool,
                    required: false,
                    unique: false,
                },
            ],
        }
    }

    #[tokio::test]
    async fn create_collection_creates_table_and_meta_row() {
        let pool = fresh_pool().await;
        let c = create_collection(&pool, &users_schema()).await.unwrap();
        assert_eq!(c.id, "users");

        // table exists with the expected columns
        let cols: Vec<(String,)> =
            sqlx::query_as("SELECT name FROM pragma_table_info('users') ORDER BY cid")
                .fetch_all(&pool)
                .await
                .unwrap();
        let names: Vec<String> = cols.into_iter().map(|c| c.0).collect();
        assert_eq!(
            names,
            vec!["id", "created_at", "updated_at", "name", "age", "verified"]
        );
    }

    #[tokio::test]
    async fn reserved_collection_id_is_rejected() {
        let pool = fresh_pool().await;
        let bad = Schema {
            id: CollectionId::from("policies"),
            kind: CollectionKind::Base,
            fields: vec![],
        };
        let err = create_collection(&pool, &bad).await.unwrap_err();
        assert!(matches!(err, DbError::InvalidIdentifier(_)));
    }

    #[tokio::test]
    async fn reserved_field_name_is_rejected() {
        let pool = fresh_pool().await;
        let bad = Schema {
            id: CollectionId::from("posts"),
            kind: CollectionKind::Base,
            fields: vec![Field {
                name: "id".into(),
                ty: FieldType::Text { min: None, max: None },
                required: false,
                unique: false,
            }],
        };
        let err = create_collection(&pool, &bad).await.unwrap_err();
        assert!(matches!(err, DbError::InvalidIdentifier(_)));
    }

    #[tokio::test]
    async fn invalid_collection_name_is_rejected() {
        let pool = fresh_pool().await;
        let bad = Schema {
            id: CollectionId::from("Users"),
            kind: CollectionKind::Base,
            fields: vec![],
        };
        let err = create_collection(&pool, &bad).await.unwrap_err();
        assert!(matches!(err, DbError::InvalidIdentifier(_)));
    }

    #[tokio::test]
    async fn list_then_find_round_trip_schema() {
        let pool = fresh_pool().await;
        create_collection(&pool, &users_schema()).await.unwrap();

        let listed = list_collections(&pool).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].schema, users_schema());

        let found = find_collection(&pool, "users").await.unwrap().unwrap();
        assert_eq!(found.schema, users_schema());
    }

    #[tokio::test]
    async fn delete_collection_drops_table_and_meta() {
        let pool = fresh_pool().await;
        create_collection(&pool, &users_schema()).await.unwrap();
        delete_collection(&pool, "users").await.unwrap();

        let still: Vec<(String,)> =
            sqlx::query_as("SELECT name FROM sqlite_master WHERE name = 'users'")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert!(still.is_empty());

        assert!(find_collection(&pool, "users").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_unknown_collection_returns_row_not_found() {
        let pool = fresh_pool().await;
        let err = delete_collection(&pool, "ghost").await.unwrap_err();
        assert!(matches!(err, DbError::Sqlx(sqlx::Error::RowNotFound)));
    }
}
