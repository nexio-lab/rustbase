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

    /// Evaluate the filter against an in-memory record's fields. Used
    /// by the realtime broker to filter events before they reach an
    /// SSE / WebSocket subscriber — the same AST that the SQL
    /// translator consumes also evaluates here so behaviour stays
    /// consistent across the two paths.
    ///
    /// Numeric / boolean / string comparisons follow `serde_json`'s
    /// natural ordering; type mismatches (`Eq("name", json!(42))` on
    /// a string column, say) yield `false` rather than an error.
    /// Special-cases nothing — a missing field is just `Value::Null`,
    /// so `Eq("x", null)` matches rows whose `x` is null AND rows
    /// without an `x` at all.
    pub fn matches(&self, fields: &std::collections::BTreeMap<String, Value>) -> bool {
        match self {
            FilterNode::And(l, r) => l.matches(fields) && r.matches(fields),
            FilterNode::Or(l, r) => l.matches(fields) || r.matches(fields),
            FilterNode::Not(inner) => !inner.matches(fields),
            FilterNode::Eq(k, v) => values_equal(fields.get(k).unwrap_or(&Value::Null), v),
            FilterNode::Ne(k, v) => !values_equal(fields.get(k).unwrap_or(&Value::Null), v),
            FilterNode::Gt(k, v) => cmp_values(fields.get(k).unwrap_or(&Value::Null), v)
                .map(|o| o.is_gt())
                .unwrap_or(false),
            FilterNode::Gte(k, v) => cmp_values(fields.get(k).unwrap_or(&Value::Null), v)
                .map(|o| !o.is_lt())
                .unwrap_or(false),
            FilterNode::Lt(k, v) => cmp_values(fields.get(k).unwrap_or(&Value::Null), v)
                .map(|o| o.is_lt())
                .unwrap_or(false),
            FilterNode::Lte(k, v) => cmp_values(fields.get(k).unwrap_or(&Value::Null), v)
                .map(|o| !o.is_gt())
                .unwrap_or(false),
            FilterNode::Like(k, pat) => match fields.get(k) {
                Some(Value::String(s)) => like_match(s, pat),
                _ => false,
            },
            FilterNode::In(k, list) => {
                let actual = fields.get(k).unwrap_or(&Value::Null);
                list.iter().any(|v| values_equal(actual, v))
            }
        }
    }
}

/// Two `serde_json::Value`s are equal for filter purposes when they
/// compare equal via `PartialEq`. Numbers stored as different inner
/// kinds (i64 vs f64) still compare via `serde_json::Number`'s impl.
fn values_equal(a: &Value, b: &Value) -> bool {
    a == b
}

/// Ordering for comparable `Value` pairs (numbers + strings). Returns
/// `None` for incomparable pairs so the caller can fall back to
/// `false`. Booleans aren't strictly ordered in JSON but we treat
/// `false < true` to give predictable behaviour on `bool` columns.
fn cmp_values(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(xf), Some(yf)) => xf.partial_cmp(&yf),
            _ => None,
        },
        (Value::String(x), Value::String(y)) => Some(x.cmp(y)),
        (Value::Bool(x), Value::Bool(y)) => Some(match (x, y) {
            (false, false) | (true, true) => Ordering::Equal,
            (false, true) => Ordering::Less,
            (true, false) => Ordering::Greater,
        }),
        _ => None,
    }
}

/// SQL-flavoured LIKE: `%` matches any run of characters, `_` matches
/// exactly one. Case-sensitive — same shape as the SQL translator's
/// `LIKE`.
fn like_match(value: &str, pattern: &str) -> bool {
    // Walk the pattern + value greedily; backtrack on `%`.
    let v: Vec<char> = value.chars().collect();
    let p: Vec<char> = pattern.chars().collect();
    fn go(v: &[char], p: &[char]) -> bool {
        if p.is_empty() {
            return v.is_empty();
        }
        match p[0] {
            '%' => {
                // Zero-or-more — try each tail.
                if go(v, &p[1..]) {
                    return true;
                }
                if v.is_empty() {
                    return false;
                }
                go(&v[1..], p)
            }
            '_' => !v.is_empty() && go(&v[1..], &p[1..]),
            ch => !v.is_empty() && v[0] == ch && go(&v[1..], &p[1..]),
        }
    }
    go(&v, &p)
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

    fn fields(
        pairs: &[(&str, serde_json::Value)],
    ) -> std::collections::BTreeMap<String, serde_json::Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn eq_matches_value() {
        let n = FilterNode::Eq("status".into(), json!("open"));
        assert!(n.matches(&fields(&[("status", json!("open"))])));
        assert!(!n.matches(&fields(&[("status", json!("closed"))])));
        // Missing column → Eq(null) ↔ Eq with null literal.
        assert!(!n.matches(&fields(&[])));
        let n_null = FilterNode::Eq("status".into(), json!(null));
        assert!(n_null.matches(&fields(&[])));
    }

    #[test]
    fn ne_is_complement_of_eq() {
        let n = FilterNode::Ne("status".into(), json!("open"));
        assert!(!n.matches(&fields(&[("status", json!("open"))])));
        assert!(n.matches(&fields(&[("status", json!("closed"))])));
    }

    #[test]
    fn gt_gte_lt_lte_on_numbers() {
        let gt = FilterNode::Gt("age".into(), json!(18));
        assert!(gt.matches(&fields(&[("age", json!(19))])));
        assert!(!gt.matches(&fields(&[("age", json!(18))])));
        let gte = FilterNode::Gte("age".into(), json!(18));
        assert!(gte.matches(&fields(&[("age", json!(18))])));
        let lte = FilterNode::Lte("age".into(), json!(18));
        assert!(lte.matches(&fields(&[("age", json!(18))])));
        assert!(!lte.matches(&fields(&[("age", json!(19))])));
    }

    #[test]
    fn cmp_with_incompatible_types_is_false() {
        let gt = FilterNode::Gt("age".into(), json!(18));
        assert!(!gt.matches(&fields(&[("age", json!("eighteen"))])));
    }

    #[test]
    fn like_pattern_with_percent_and_underscore() {
        let n = FilterNode::Like("name".into(), "ali_e%".into());
        assert!(n.matches(&fields(&[("name", json!("alice"))])));
        assert!(n.matches(&fields(&[("name", json!("aline-foo"))])));
        assert!(!n.matches(&fields(&[("name", json!("bob"))])));
    }

    #[test]
    fn in_matches_any_member() {
        let n = FilterNode::In("kind".into(), vec![json!("a"), json!("b")]);
        assert!(n.matches(&fields(&[("kind", json!("a"))])));
        assert!(n.matches(&fields(&[("kind", json!("b"))])));
        assert!(!n.matches(&fields(&[("kind", json!("c"))])));
    }

    #[test]
    fn compound_and_or_not() {
        // status = 'open' AND age > 17
        let n = FilterNode::and(
            FilterNode::Eq("status".into(), json!("open")),
            FilterNode::Gt("age".into(), json!(17)),
        );
        assert!(n.matches(&fields(&[("status", json!("open")), ("age", json!(18))])));
        assert!(!n.matches(&fields(&[("status", json!("closed")), ("age", json!(18))])));
        // NOT (status = 'open')
        let neg = FilterNode::not(FilterNode::Eq("status".into(), json!("open")));
        assert!(neg.matches(&fields(&[("status", json!("closed"))])));
    }
}
