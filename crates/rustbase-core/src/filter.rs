use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Parsed filter expression. IO-free and dialect-agnostic; the SQL
/// translator lives in `rustbase-db`, and the dashboard reuses this AST
/// for client-side validation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FilterNode {
    And(Box<FilterNode>, Box<FilterNode>),
    Or(Box<FilterNode>, Box<FilterNode>),
    Not(Box<FilterNode>),
    Eq(String, Value),
    Ne(String, Value),
    Gt(String, Value),
    Gte(String, Value),
    Lt(String, Value),
    Lte(String, Value),
    Like(String, String),
    In(String, Vec<Value>),
}

impl FilterNode {
    pub fn and(lhs: FilterNode, rhs: FilterNode) -> Self {
        FilterNode::And(Box::new(lhs), Box::new(rhs))
    }

    pub fn or(lhs: FilterNode, rhs: FilterNode) -> Self {
        FilterNode::Or(Box::new(lhs), Box::new(rhs))
    }

    // Companion constructor for the AST variant — mirrors `and` / `or`
    // above. Not an impl of std::ops::Not, which would force `!value`
    // syntax and lose the explicit construction pattern.
    #[allow(clippy::should_implement_trait)]
    pub fn not(inner: FilterNode) -> Self {
        FilterNode::Not(Box::new(inner))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_compound_expression() {
        let node = FilterNode::and(
            FilterNode::Eq("status".into(), json!("active")),
            FilterNode::Gt("age".into(), json!(18)),
        );
        match node {
            FilterNode::And(lhs, rhs) => {
                assert!(matches!(*lhs, FilterNode::Eq(_, _)));
                assert!(matches!(*rhs, FilterNode::Gt(_, _)));
            }
            _ => panic!("expected And"),
        }
    }

    #[test]
    fn serde_round_trip() {
        let node = FilterNode::or(
            FilterNode::Eq("kind".into(), json!("a")),
            FilterNode::Eq("kind".into(), json!("b")),
        );
        let json = serde_json::to_string(&node).unwrap();
        let parsed: FilterNode = serde_json::from_str(&json).unwrap();
        assert_eq!(node, parsed);
    }
}
