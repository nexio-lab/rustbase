use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! id_type {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(s: impl Into<String>) -> Self {
                Self(s.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_string())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

id_type!(
    WorkspaceId,
    "Identifier for a workspace (identity / organization boundary)."
);
id_type!(AppId, "Identifier for an app within a workspace.");
id_type!(UserId, "Identifier for an end user within a workspace.");
id_type!(AdminId, "Identifier for a master, workspace, or app admin.");
id_type!(CollectionId, "Identifier for a collection within an app.");
id_type!(RecordId, "Identifier for a record within a collection.");

/// Reserved id of the master workspace. It cannot be deleted.
pub const MASTER_WORKSPACE_ID: &str = "master";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_id_round_trips_through_json() {
        let id = WorkspaceId::new("acme");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"acme\"");
        let parsed: WorkspaceId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn id_types_are_distinct() {
        // This is a compile-time guarantee — WorkspaceId and AppId
        // cannot be assigned to each other. We just smoke-test the
        // constructors.
        let w = WorkspaceId::from("acme");
        let a = AppId::from("mobile");
        assert_eq!(w.as_str(), "acme");
        assert_eq!(a.as_str(), "mobile");
    }
}
