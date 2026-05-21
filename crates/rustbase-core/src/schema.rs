use crate::id::CollectionId;
use serde::{Deserialize, Serialize};

/// Schema for a single collection inside an app.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Schema {
    pub id: CollectionId,
    pub kind: CollectionKind,
    pub fields: Vec<Field>,
}

/// Three flavours of collections, matching the design spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollectionKind {
    /// A plain collection of records.
    Base,
    /// A collection whose records authenticate. Auto-includes
    /// `email`, `password_hash`, `verified`, `last_login`, `oauth_providers`.
    Auth,
    /// A read-only collection backed by a SQL query against other collections.
    View,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field {
    pub name: String,
    #[serde(flatten)]
    pub ty: FieldType,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub unique: bool,
}

/// Typed shape of a field. Tagged JSON shape: `{"kind": "text", ...}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FieldType {
    Text {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<u32>,
    },
    Number {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
    },
    Bool,
    Email,
    Url,
    Date,
    Json,
    Relation {
        target: CollectionId,
        #[serde(default)]
        cascade_delete: bool,
    },
    File {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_size: Option<u64>,
        #[serde(default)]
        mime_types: Vec<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_schema_serializes() {
        let schema = Schema {
            id: CollectionId::from("users"),
            kind: CollectionKind::Auth,
            fields: vec![Field {
                name: "name".into(),
                ty: FieldType::Text {
                    min: Some(1),
                    max: Some(100),
                },
                required: true,
                unique: false,
            }],
        };
        let json = serde_json::to_string(&schema).unwrap();
        assert!(json.contains("\"kind\":\"auth\""));
        assert!(json.contains("\"kind\":\"text\""));
        let parsed: Schema = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, schema);
    }

    #[test]
    fn relation_field_round_trip() {
        let field = Field {
            name: "owner".into(),
            ty: FieldType::Relation {
                target: CollectionId::from("users"),
                cascade_delete: true,
            },
            required: true,
            unique: false,
        };
        let json = serde_json::to_string(&field).unwrap();
        let parsed: Field = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, field);
    }
}
