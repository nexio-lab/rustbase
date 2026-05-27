//! Record CRUD for user-defined collections.
//!
//! Records live in per-collection tables (named exactly after the
//! collection's id, which is slug-validated). Field values are bound /
//! extracted with `sqlx::QueryBuilder` so we can ship arbitrary
//! schemas without monomorphising a query per shape:
//!
//!   - `Text`/`Email`/`Url`/`Date`/`Relation`/`File` → SQLite `TEXT`
//!   - `Number` → `REAL`
//!   - `Bool` → `INTEGER` (0/1)
//!   - `Json` → `TEXT` (serialised JSON)
//!
//! Identifier safety is enforced at collection-create time
//! (`collections::validate_schema`); this module assumes the caller
//! hands it a stored `Schema` whose names already passed those checks.

use crate::error::{DbError, Result};
use crate::filter_sql::filter_to_sql;
use chrono::{DateTime, Utc};
use rustbase_core::{FieldType, FilterNode, Record, RecordId, Schema};
use serde_json::Value as Json;
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool, sqlite::SqliteRow};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Debug, Clone, Copy)]
pub struct ListPage {
    pub page: u32,
    pub per_page: u32,
}

impl Default for ListPage {
    fn default() -> Self {
        Self {
            page: 1,
            per_page: 30,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ListedRecords {
    pub items: Vec<Record>,
    pub page: u32,
    pub per_page: u32,
    pub total_items: u64,
}

pub async fn create_record(
    pool: &SqlitePool,
    schema: &Schema,
    fields: BTreeMap<String, Json>,
) -> Result<Record> {
    let id = Uuid::now_v7().to_string();
    let now = Utc::now();

    let mut q: QueryBuilder<Sqlite> = QueryBuilder::new("INSERT INTO ");
    q.push(quote_ident(schema.id.as_str()));
    q.push(" (id, created_at, updated_at");
    for f in &schema.fields {
        q.push(", ");
        q.push(quote_ident(&f.name));
    }
    q.push(") VALUES (");
    q.push_bind(id.clone());
    q.push(", ");
    q.push_bind(now);
    q.push(", ");
    q.push_bind(now);
    for f in &schema.fields {
        q.push(", ");
        push_field_value(&mut q, fields.get(&f.name), &f.ty);
    }
    q.push(")");
    q.build().execute(pool).await?;

    Ok(Record {
        id: RecordId::from(id),
        collection: schema.id.clone(),
        fields,
        created_at: now,
        updated_at: now,
    })
}

pub async fn find_record(pool: &SqlitePool, schema: &Schema, id: &str) -> Result<Option<Record>> {
    let sql = format!(
        "SELECT * FROM {} WHERE id = ?",
        quote_ident(schema.id.as_str())
    );
    let row: Option<SqliteRow> = sqlx::query(&sql).bind(id).fetch_optional(pool).await?;
    row.map(|r| row_to_record(&r, schema)).transpose()
}

pub async fn list_records(
    pool: &SqlitePool,
    schema: &Schema,
    page: ListPage,
    filter: Option<&FilterNode>,
) -> Result<ListedRecords> {
    let per_page = page.per_page.clamp(1, 200);
    let page_num = page.page.max(1);
    let offset = ((page_num - 1) as i64) * (per_page as i64);

    let (where_clause, bindings): (String, Vec<Json>) = match filter {
        Some(f) => {
            let frag = filter_to_sql(f)?;
            (format!(" WHERE {}", frag.sql), frag.bindings)
        }
        None => (String::new(), vec![]),
    };

    // --- count ---
    let count_sql = format!(
        "SELECT COUNT(*) FROM {}{}",
        quote_ident(schema.id.as_str()),
        where_clause
    );
    let mut count_q = sqlx::query_scalar::<_, i64>(&count_sql);
    for v in &bindings {
        count_q = bind_filter_value_scalar(count_q, v);
    }
    let total_items: i64 = count_q.fetch_one(pool).await?;

    // --- page ---
    let list_sql = format!(
        "SELECT * FROM {}{} ORDER BY created_at DESC LIMIT ? OFFSET ?",
        quote_ident(schema.id.as_str()),
        where_clause
    );
    let mut list_q = sqlx::query(&list_sql);
    for v in &bindings {
        list_q = bind_filter_value(list_q, v);
    }
    list_q = list_q.bind(per_page as i64).bind(offset);
    let rows: Vec<SqliteRow> = list_q.fetch_all(pool).await?;
    let items: Vec<Record> = rows
        .iter()
        .map(|r| row_to_record(r, schema))
        .collect::<Result<_>>()?;

    Ok(ListedRecords {
        items,
        page: page_num,
        per_page,
        total_items: total_items.max(0) as u64,
    })
}

fn bind_filter_value<'a>(
    q: sqlx::query::Query<'a, Sqlite, sqlx::sqlite::SqliteArguments<'a>>,
    v: &Json,
) -> sqlx::query::Query<'a, Sqlite, sqlx::sqlite::SqliteArguments<'a>> {
    match v {
        Json::String(s) => q.bind(s.clone()),
        Json::Bool(b) => q.bind(*b),
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                q.bind(i)
            } else if let Some(f) = n.as_f64() {
                q.bind(f)
            } else {
                q.bind(Option::<i64>::None)
            }
        }
        Json::Null => q.bind(Option::<String>::None),
        other => q.bind(other.to_string()),
    }
}

fn bind_filter_value_scalar<'a>(
    q: sqlx::query::QueryScalar<'a, Sqlite, i64, sqlx::sqlite::SqliteArguments<'a>>,
    v: &Json,
) -> sqlx::query::QueryScalar<'a, Sqlite, i64, sqlx::sqlite::SqliteArguments<'a>> {
    match v {
        Json::String(s) => q.bind(s.clone()),
        Json::Bool(b) => q.bind(*b),
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                q.bind(i)
            } else if let Some(f) = n.as_f64() {
                q.bind(f)
            } else {
                q.bind(Option::<i64>::None)
            }
        }
        Json::Null => q.bind(Option::<String>::None),
        other => q.bind(other.to_string()),
    }
}

pub async fn update_record(
    pool: &SqlitePool,
    schema: &Schema,
    id: &str,
    patch: BTreeMap<String, Json>,
) -> Result<Record> {
    // Only update fields that the patch supplies AND that the schema knows.
    let touched: Vec<&rustbase_core::Field> = schema
        .fields
        .iter()
        .filter(|f| patch.contains_key(&f.name))
        .collect();

    let now = Utc::now();
    let mut q: QueryBuilder<Sqlite> = QueryBuilder::new("UPDATE ");
    q.push(quote_ident(schema.id.as_str()));
    q.push(" SET updated_at = ");
    q.push_bind(now);
    for f in &touched {
        q.push(", ");
        q.push(quote_ident(&f.name));
        q.push(" = ");
        push_field_value(&mut q, patch.get(&f.name), &f.ty);
    }
    q.push(" WHERE id = ");
    q.push_bind(id.to_string());
    let res = q.build().execute(pool).await?;
    if res.rows_affected() == 0 {
        return Err(DbError::Sqlx(sqlx::Error::RowNotFound));
    }

    find_record(pool, schema, id)
        .await?
        .ok_or(DbError::Sqlx(sqlx::Error::RowNotFound))
}

pub async fn delete_record(pool: &SqlitePool, schema: &Schema, id: &str) -> Result<()> {
    let sql = format!(
        "DELETE FROM {} WHERE id = ?",
        quote_ident(schema.id.as_str())
    );
    let res = sqlx::query(&sql).bind(id).execute(pool).await?;
    if res.rows_affected() == 0 {
        return Err(DbError::Sqlx(sqlx::Error::RowNotFound));
    }
    Ok(())
}

// -----------------------------------------------------------------------------

fn quote_ident(s: &str) -> String {
    format!("\"{s}\"")
}

fn push_field_value(q: &mut QueryBuilder<'_, Sqlite>, value: Option<&Json>, ty: &FieldType) {
    match ty {
        FieldType::Bool => {
            let v: Option<i64> = value.and_then(Json::as_bool).map(|b| b as i64);
            q.push_bind(v);
        }
        FieldType::Number { .. } => {
            let v: Option<f64> = value.and_then(Json::as_f64);
            q.push_bind(v);
        }
        FieldType::Json => {
            let v: Option<String> = value.and_then(|v| {
                if v.is_null() {
                    None
                } else {
                    Some(v.to_string())
                }
            });
            q.push_bind(v);
        }
        FieldType::Text { .. }
        | FieldType::Email
        | FieldType::Url
        | FieldType::Date
        | FieldType::Relation { .. }
        | FieldType::File { .. } => {
            let v: Option<String> = value.and_then(|v| match v {
                Json::String(s) => Some(s.clone()),
                Json::Null => None,
                other => Some(other.to_string()),
            });
            q.push_bind(v);
        }
    }
}

fn row_to_record(row: &SqliteRow, schema: &Schema) -> Result<Record> {
    let id: String = row.try_get::<String, _>("id").map_err(DbError::Sqlx)?;
    let created_at: DateTime<Utc> = row
        .try_get::<DateTime<Utc>, _>("created_at")
        .map_err(DbError::Sqlx)?;
    let updated_at: DateTime<Utc> = row
        .try_get::<DateTime<Utc>, _>("updated_at")
        .map_err(DbError::Sqlx)?;

    let mut fields = BTreeMap::new();
    for f in &schema.fields {
        let val = extract_field(row, &f.name, &f.ty);
        fields.insert(f.name.clone(), val);
    }

    Ok(Record {
        id: RecordId::from(id),
        collection: schema.id.clone(),
        fields,
        created_at,
        updated_at,
    })
}

fn extract_field(row: &SqliteRow, name: &str, ty: &FieldType) -> Json {
    match ty {
        FieldType::Bool => match row.try_get::<Option<i64>, _>(name) {
            Ok(Some(v)) => Json::Bool(v != 0),
            _ => Json::Null,
        },
        FieldType::Number { .. } => match row.try_get::<Option<f64>, _>(name) {
            Ok(Some(v)) => serde_json::Number::from_f64(v)
                .map(Json::Number)
                .unwrap_or(Json::Null),
            _ => Json::Null,
        },
        FieldType::Json => match row.try_get::<Option<String>, _>(name) {
            Ok(Some(s)) => serde_json::from_str(&s).unwrap_or(Json::Null),
            _ => Json::Null,
        },
        _ => match row.try_get::<Option<String>, _>(name) {
            Ok(Some(s)) => Json::String(s),
            _ => Json::Null,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collections::create_collection;
    use crate::migrations::{APP_MIGRATIONS, apply_migrations};
    use crate::pool::open_memory_pool;
    use rustbase_core::{CollectionId, CollectionKind, Field, FieldType, Schema};
    use serde_json::json;

    async fn pool_with_users() -> (SqlitePool, Schema) {
        let pool = open_memory_pool().await.unwrap();
        apply_migrations(pool.clone(), APP_MIGRATIONS)
            .await
            .unwrap();
        let schema = Schema {
            id: CollectionId::from("people"),
            kind: CollectionKind::Base,
            fields: vec![
                Field {
                    name: "name".into(),
                    ty: FieldType::Text {
                        min: None,
                        max: None,
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
                Field {
                    name: "tags".into(),
                    ty: FieldType::Json,
                    required: false,
                    unique: false,
                },
            ],
        };
        create_collection(&pool, &schema).await.unwrap();
        (pool, schema)
    }

    fn fields() -> BTreeMap<String, Json> {
        let mut m = BTreeMap::new();
        m.insert("name".into(), json!("Ada"));
        m.insert("age".into(), json!(36));
        m.insert("verified".into(), json!(true));
        m.insert("tags".into(), json!(["pioneer", "math"]));
        m
    }

    #[tokio::test]
    async fn create_then_find_round_trips_typed_fields() {
        let (pool, schema) = pool_with_users().await;
        let created = create_record(&pool, &schema, fields()).await.unwrap();

        let loaded = find_record(&pool, &schema, created.id.as_str())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.fields.get("name"), Some(&json!("Ada")));
        assert_eq!(loaded.fields.get("verified"), Some(&json!(true)));
        // age came back as a JSON number; compare via f64 to be tolerant
        // of integer-vs-float JSON representation.
        let age = loaded.fields.get("age").and_then(|v| v.as_f64()).unwrap();
        assert!((age - 36.0).abs() < f64::EPSILON);
        assert_eq!(loaded.fields.get("tags"), Some(&json!(["pioneer", "math"])));
    }

    #[tokio::test]
    async fn list_records_paginates_and_counts() {
        let (pool, schema) = pool_with_users().await;
        for i in 0..5u32 {
            let mut f = fields();
            f.insert("name".into(), json!(format!("user-{i}")));
            create_record(&pool, &schema, f).await.unwrap();
        }
        let listed = list_records(
            &pool,
            &schema,
            ListPage {
                page: 1,
                per_page: 2,
            },
            None,
        )
        .await
        .unwrap();
        assert_eq!(listed.items.len(), 2);
        assert_eq!(listed.total_items, 5);
        assert_eq!(listed.per_page, 2);
        assert_eq!(listed.page, 1);
    }

    #[tokio::test]
    async fn update_record_only_touches_supplied_fields() {
        let (pool, schema) = pool_with_users().await;
        let created = create_record(&pool, &schema, fields()).await.unwrap();

        let mut patch = BTreeMap::new();
        patch.insert("name".into(), json!("Ada Lovelace"));
        let updated = update_record(&pool, &schema, created.id.as_str(), patch)
            .await
            .unwrap();
        assert_eq!(updated.fields.get("name"), Some(&json!("Ada Lovelace")));
        // verified should still be true (untouched by the patch)
        assert_eq!(updated.fields.get("verified"), Some(&json!(true)));
        // updated_at moved forward
        assert!(updated.updated_at >= created.updated_at);
    }

    #[tokio::test]
    async fn update_unknown_returns_row_not_found() {
        let (pool, schema) = pool_with_users().await;
        let err = update_record(&pool, &schema, "no-such-id", BTreeMap::new())
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::Sqlx(sqlx::Error::RowNotFound)));
    }

    #[tokio::test]
    async fn delete_record_removes_row() {
        let (pool, schema) = pool_with_users().await;
        let created = create_record(&pool, &schema, fields()).await.unwrap();
        delete_record(&pool, &schema, created.id.as_str())
            .await
            .unwrap();
        assert!(
            find_record(&pool, &schema, created.id.as_str())
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn list_with_filter_matches_only_matching_rows() {
        let (pool, schema) = pool_with_users().await;
        for (name, age, pinned) in [
            ("Ada", 36, true),
            ("Babbage", 80, false),
            ("Lovelace", 36, true),
        ] {
            let mut f = BTreeMap::new();
            f.insert("name".into(), json!(name));
            f.insert("age".into(), json!(age));
            f.insert("verified".into(), json!(pinned));
            create_record(&pool, &schema, f).await.unwrap();
        }

        let filter = rustbase_core::parse_filter("age = 36 && verified = true").unwrap();
        let listed = list_records(&pool, &schema, ListPage::default(), Some(&filter))
            .await
            .unwrap();
        assert_eq!(listed.total_items, 2);
        assert_eq!(listed.items.len(), 2);
        // both returned names are "Ada" or "Lovelace"
        for r in &listed.items {
            let name = r.fields.get("name").and_then(|v| v.as_str()).unwrap();
            assert!(name == "Ada" || name == "Lovelace");
        }
    }

    #[tokio::test]
    async fn list_with_like_filter_matches_substring() {
        let (pool, schema) = pool_with_users().await;
        for n in ["Ada Lovelace", "Charles Babbage", "Grace Hopper"] {
            let mut f = BTreeMap::new();
            f.insert("name".into(), json!(n));
            create_record(&pool, &schema, f).await.unwrap();
        }
        let filter = rustbase_core::parse_filter(r#"name ~ "ace""#).unwrap();
        let listed = list_records(&pool, &schema, ListPage::default(), Some(&filter))
            .await
            .unwrap();
        // "Lovelace" and "Grace" both contain "ace"
        assert_eq!(listed.total_items, 2);
    }

    #[tokio::test]
    async fn list_filter_with_no_matches_returns_empty() {
        let (pool, schema) = pool_with_users().await;
        let mut f = BTreeMap::new();
        f.insert("name".into(), json!("solo"));
        create_record(&pool, &schema, f).await.unwrap();
        let filter = rustbase_core::parse_filter(r#"name = "nope""#).unwrap();
        let listed = list_records(&pool, &schema, ListPage::default(), Some(&filter))
            .await
            .unwrap();
        assert_eq!(listed.total_items, 0);
        assert!(listed.items.is_empty());
    }

    #[tokio::test]
    async fn null_input_for_optional_fields_stored_as_null() {
        let (pool, schema) = pool_with_users().await;
        let mut f = BTreeMap::new();
        f.insert("name".into(), json!("X"));
        // age, verified, tags intentionally omitted
        let created = create_record(&pool, &schema, f).await.unwrap();
        let loaded = find_record(&pool, &schema, created.id.as_str())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.fields.get("age"), Some(&Json::Null));
        assert_eq!(loaded.fields.get("verified"), Some(&Json::Null));
        assert_eq!(loaded.fields.get("tags"), Some(&Json::Null));
    }
}
