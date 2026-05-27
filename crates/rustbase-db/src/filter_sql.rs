//! Translate a `FilterNode` AST into a parameterized SQLite `WHERE`
//! fragment.
//!
//! No user-supplied value is ever interpolated into the SQL string —
//! every literal becomes a bound parameter on the returned `SqlFragment`.
//! Field identifiers are validated against an allowlist (`[A-Za-z0-9_]`)
//! before being quoted; dotted paths (`user.email`) are rejected here
//! because they imply a JOIN that the records layer hasn't taught us yet.

use crate::error::{DbError, Result};
use rustbase_core::filter::FilterNode;
use serde_json::Value;

/// A parameterized SQL fragment ready to be appended after `WHERE`.
#[derive(Debug, Default)]
pub struct SqlFragment {
    pub sql: String,
    pub bindings: Vec<Value>,
}

/// Translate a filter into a parameterized SQL fragment.
pub fn filter_to_sql(filter: &FilterNode) -> Result<SqlFragment> {
    let mut out = SqlFragment::default();
    translate(filter, &mut out)?;
    Ok(out)
}

fn translate(node: &FilterNode, out: &mut SqlFragment) -> Result<()> {
    match node {
        FilterNode::And(lhs, rhs) => binary(out, lhs, "AND", rhs),
        FilterNode::Or(lhs, rhs) => binary(out, lhs, "OR", rhs),
        FilterNode::Not(inner) => {
            out.sql.push_str("(NOT ");
            translate(inner, out)?;
            out.sql.push(')');
            Ok(())
        }
        FilterNode::Eq(field, value) => compare(out, field, "=", value),
        FilterNode::Ne(field, value) => compare(out, field, "<>", value),
        FilterNode::Gt(field, value) => compare(out, field, ">", value),
        FilterNode::Gte(field, value) => compare(out, field, ">=", value),
        FilterNode::Lt(field, value) => compare(out, field, "<", value),
        FilterNode::Lte(field, value) => compare(out, field, "<=", value),
        FilterNode::Like(field, pat) => {
            let col = quote_ident(field)?;
            out.sql.push('(');
            out.sql.push_str(&col);
            out.sql.push_str(" LIKE ?)");
            out.bindings.push(Value::String(format!("%{pat}%")));
            Ok(())
        }
        FilterNode::In(field, values) => {
            let col = quote_ident(field)?;
            out.sql.push('(');
            out.sql.push_str(&col);
            out.sql.push_str(" IN (");
            for (i, v) in values.iter().enumerate() {
                if i > 0 {
                    out.sql.push_str(", ");
                }
                out.sql.push('?');
                out.bindings.push(v.clone());
            }
            out.sql.push_str("))");
            Ok(())
        }
    }
}

fn binary(out: &mut SqlFragment, lhs: &FilterNode, op: &str, rhs: &FilterNode) -> Result<()> {
    out.sql.push('(');
    translate(lhs, out)?;
    out.sql.push(' ');
    out.sql.push_str(op);
    out.sql.push(' ');
    translate(rhs, out)?;
    out.sql.push(')');
    Ok(())
}

fn compare(out: &mut SqlFragment, field: &str, op: &str, value: &Value) -> Result<()> {
    let col = quote_ident(field)?;
    out.sql.push('(');
    out.sql.push_str(&col);
    out.sql.push(' ');
    out.sql.push_str(op);
    out.sql.push_str(" ?)");
    out.bindings.push(value.clone());
    Ok(())
}

/// Quote a column identifier for SQLite, validating the input as
/// `[A-Za-z_][A-Za-z0-9_]*` first. Dotted paths are rejected here
/// because they imply a JOIN that this layer cannot synthesize yet.
fn quote_ident(field: &str) -> Result<String> {
    let mut chars = field.chars();
    let Some(first) = chars.next() else {
        return Err(DbError::InvalidIdentifier(field.to_string()));
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(DbError::InvalidIdentifier(field.to_string()));
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(DbError::InvalidIdentifier(field.to_string()));
    }
    Ok(format!("\"{field}\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustbase_core::parse_filter;
    use serde_json::json;

    #[test]
    fn simple_equality_produces_one_binding() {
        let node = parse_filter(r#"status = "active""#).unwrap();
        let frag = filter_to_sql(&node).unwrap();
        assert_eq!(frag.sql, r#"("status" = ?)"#);
        assert_eq!(frag.bindings, vec![json!("active")]);
    }

    #[test]
    fn compound_expression_with_parens() {
        let node = parse_filter("a = 1 && b = 2").unwrap();
        let frag = filter_to_sql(&node).unwrap();
        assert_eq!(frag.sql, r#"(("a" = ?) AND ("b" = ?))"#);
        assert_eq!(frag.bindings, vec![json!(1), json!(2)]);
    }

    #[test]
    fn not_prefix_wraps_inner() {
        let node = parse_filter("verified = true").unwrap();
        let not = FilterNode::not(node);
        let frag = filter_to_sql(&not).unwrap();
        assert_eq!(frag.sql, r#"(NOT ("verified" = ?))"#);
    }

    #[test]
    fn like_wraps_pattern_with_percents() {
        let node = parse_filter(r#"name ~ "Ada""#).unwrap();
        let frag = filter_to_sql(&node).unwrap();
        assert_eq!(frag.sql, r#"("name" LIKE ?)"#);
        assert_eq!(frag.bindings, vec![json!("%Ada%")]);
    }

    #[test]
    fn in_emits_placeholder_per_value() {
        let node = FilterNode::In("kind".into(), vec![json!("a"), json!("b"), json!("c")]);
        let frag = filter_to_sql(&node).unwrap();
        assert_eq!(frag.sql, r#"("kind" IN (?, ?, ?))"#);
        assert_eq!(frag.bindings, vec![json!("a"), json!("b"), json!("c")]);
    }

    #[test]
    fn rejects_dotted_identifier_for_now() {
        let node = parse_filter(r#"user.email = "x""#).unwrap();
        let err = filter_to_sql(&node).unwrap_err();
        assert!(matches!(err, DbError::InvalidIdentifier(_)));
    }

    #[test]
    fn rejects_quote_in_identifier() {
        let node = FilterNode::Eq("evil\"col".into(), json!(1));
        let err = filter_to_sql(&node).unwrap_err();
        assert!(matches!(err, DbError::InvalidIdentifier(_)));
    }

    #[test]
    fn rejects_leading_digit_identifier() {
        let node = FilterNode::Eq("1abc".into(), json!(1));
        let err = filter_to_sql(&node).unwrap_err();
        assert!(matches!(err, DbError::InvalidIdentifier(_)));
    }
}
