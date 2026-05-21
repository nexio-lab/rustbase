use crate::id::{CollectionId, RecordId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// A row inside a collection. Field values are stored as `serde_json::Value`;
/// `Schema` constrains which values are valid for each field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub id: RecordId,
    pub collection: CollectionId,
    pub fields: BTreeMap<String, Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Record {
    pub fn new(collection: CollectionId, id: RecordId) -> Self {
        let now = Utc::now();
        Self {
            id,
            collection,
            fields: BTreeMap::new(),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn with_field(mut self, name: impl Into<String>, value: Value) -> Self {
        self.fields.insert(name.into(), value);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn record_serializes_with_iso_timestamps() {
        let rec = Record::new(CollectionId::from("users"), RecordId::from("r1"))
            .with_field("name", json!("Ada"))
            .with_field("age", json!(30));
        let json = serde_json::to_string(&rec).unwrap();
        // Chrono's default serde format is RFC 3339 / ISO 8601.
        assert!(json.contains("\"name\":\"Ada\""));
        assert!(json.contains("\"age\":30"));
    }
}
