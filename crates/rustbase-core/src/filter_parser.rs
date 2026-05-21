//! `nom`-based parser for the RustBase filter language.
//!
//! Grammar (lowest to highest precedence):
//!
//! ```text
//! expr        = or_expr
//! or_expr     = and_expr ( '||' and_expr )*
//! and_expr    = not_expr ( '&&' not_expr )*
//! not_expr    = '!'? atom
//! atom        = '(' expr ')' | comparison
//! comparison  = ident OP value
//! OP          = '=' | '!=' | '>' | '>=' | '<' | '<=' | '~'
//! ident       = [A-Za-z_] [A-Za-z0-9_.]*
//! value       = string | number | 'true' | 'false' | 'null'
//! string      = '"' (any char except '"')* '"'
//! number      = -?\d+(\.\d+)?
//! ```
//!
//! All literals end up as `serde_json::Value`. Operator `~` always takes a
//! string literal on the RHS (substring LIKE match).

use crate::error::{CoreError, Result};
use crate::filter::FilterNode;
use nom::{
    IResult, Parser,
    branch::alt,
    bytes::complete::{tag, take_while, take_while1},
    character::complete::{char, multispace0},
    combinator::{map, recognize, value},
    multi::many0,
    number::complete::double,
    sequence::{delimited, pair, preceded},
};
use serde_json::Value;

/// Parse a filter expression. Returns `CoreError::Validation` on syntax errors.
pub fn parse_filter(input: &str) -> Result<FilterNode> {
    if input.trim().is_empty() {
        return Err(CoreError::Validation("empty filter".into()));
    }
    match expr(input) {
        Ok((rest, node)) if rest.trim().is_empty() => Ok(node),
        Ok((rest, _)) => Err(CoreError::Validation(format!(
            "unexpected trailing input: {rest:?}"
        ))),
        Err(e) => Err(CoreError::Validation(format!("parse error: {e}"))),
    }
}

fn expr(i: &str) -> IResult<&str, FilterNode> {
    or_expr(i)
}

fn or_expr(i: &str) -> IResult<&str, FilterNode> {
    let (i, init) = and_expr(i)?;
    let (i, rest) = many0(preceded(ws_tag("||"), and_expr)).parse(i)?;
    Ok((i, rest.into_iter().fold(init, FilterNode::or)))
}

fn and_expr(i: &str) -> IResult<&str, FilterNode> {
    let (i, init) = not_expr(i)?;
    let (i, rest) = many0(preceded(ws_tag("&&"), not_expr)).parse(i)?;
    Ok((i, rest.into_iter().fold(init, FilterNode::and)))
}

fn not_expr(i: &str) -> IResult<&str, FilterNode> {
    alt((map(preceded(ws_char('!'), atom), FilterNode::not), atom)).parse(i)
}

fn atom(i: &str) -> IResult<&str, FilterNode> {
    alt((
        delimited(ws_char('('), expr, ws_char(')')),
        comparison,
    ))
    .parse(i)
}

fn comparison(i: &str) -> IResult<&str, FilterNode> {
    let (i, _) = multispace0(i)?;
    let (i, field) = ident(i)?;
    let (i, _) = multispace0(i)?;
    let (i, op) = alt((
        tag(">="),
        tag("<="),
        tag("!="),
        tag("="),
        tag(">"),
        tag("<"),
        tag("~"),
    ))
    .parse(i)?;
    let (i, _) = multispace0(i)?;

    if op == "~" {
        let (i, pat) = string_literal(i)?;
        Ok((i, FilterNode::Like(field.to_string(), pat)))
    } else {
        let (i, val) = json_value(i)?;
        let node = match op {
            "=" => FilterNode::Eq(field.to_string(), val),
            "!=" => FilterNode::Ne(field.to_string(), val),
            ">" => FilterNode::Gt(field.to_string(), val),
            ">=" => FilterNode::Gte(field.to_string(), val),
            "<" => FilterNode::Lt(field.to_string(), val),
            "<=" => FilterNode::Lte(field.to_string(), val),
            _ => unreachable!("operator matched above"),
        };
        Ok((i, node))
    }
}

fn ident(i: &str) -> IResult<&str, &str> {
    recognize(pair(
        take_while1(|c: char| c.is_ascii_alphabetic() || c == '_'),
        take_while(|c: char| c.is_ascii_alphanumeric() || c == '_' || c == '.'),
    ))
    .parse(i)
}

fn json_value(i: &str) -> IResult<&str, Value> {
    alt((
        value(Value::Null, tag("null")),
        value(Value::Bool(true), tag("true")),
        value(Value::Bool(false), tag("false")),
        map(string_literal, Value::String),
        map(double, |n: f64| {
            if n.fract() == 0.0 && n.abs() < (i64::MAX as f64) {
                Value::from(n as i64)
            } else {
                Value::from(n)
            }
        }),
    ))
    .parse(i)
}

fn string_literal(i: &str) -> IResult<&str, String> {
    delimited(
        char('"'),
        map(take_while(|c: char| c != '"'), |s: &str| s.to_string()),
        char('"'),
    )
    .parse(i)
}

fn ws_tag(t: &'static str) -> impl Fn(&str) -> IResult<&str, &str> {
    move |i| {
        let (i, _) = multispace0(i)?;
        let (i, t) = tag(t).parse(i)?;
        let (i, _) = multispace0(i)?;
        Ok((i, t))
    }
}

fn ws_char(c: char) -> impl Fn(&str) -> IResult<&str, char> {
    move |i| {
        let (i, _) = multispace0(i)?;
        let (i, c) = char(c).parse(i)?;
        let (i, _) = multispace0(i)?;
        Ok((i, c))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn simple_equality_string() {
        let node = parse_filter(r#"status = "active""#).unwrap();
        assert_eq!(node, FilterNode::Eq("status".into(), json!("active")));
    }

    #[test]
    fn simple_equality_int() {
        let node = parse_filter("age = 18").unwrap();
        assert_eq!(node, FilterNode::Eq("age".into(), json!(18)));
    }

    #[test]
    fn negative_number_literal() {
        let node = parse_filter("delta = -3.14").unwrap();
        assert_eq!(node, FilterNode::Eq("delta".into(), json!(-3.14)));
    }

    #[test]
    fn null_literal() {
        let node = parse_filter("deleted_at = null").unwrap();
        assert_eq!(node, FilterNode::Eq("deleted_at".into(), json!(null)));
    }

    #[test]
    fn bool_literal() {
        let node = parse_filter("verified = true").unwrap();
        assert_eq!(node, FilterNode::Eq("verified".into(), json!(true)));
    }

    #[test]
    fn all_comparison_operators() {
        assert!(matches!(
            parse_filter("a != 1").unwrap(),
            FilterNode::Ne(_, _)
        ));
        assert!(matches!(
            parse_filter("a >= 1").unwrap(),
            FilterNode::Gte(_, _)
        ));
        assert!(matches!(
            parse_filter("a <= 1").unwrap(),
            FilterNode::Lte(_, _)
        ));
        assert!(matches!(
            parse_filter("a > 1").unwrap(),
            FilterNode::Gt(_, _)
        ));
        assert!(matches!(
            parse_filter("a < 1").unwrap(),
            FilterNode::Lt(_, _)
        ));
    }

    #[test]
    fn like_operator_requires_string() {
        let node = parse_filter(r#"name ~ "Ada""#).unwrap();
        assert_eq!(node, FilterNode::Like("name".into(), "Ada".into()));
    }

    #[test]
    fn dotted_field_path_for_nested_access() {
        let node = parse_filter(r#"user.email = "a@b.c""#).unwrap();
        assert_eq!(node, FilterNode::Eq("user.email".into(), json!("a@b.c")));
    }

    #[test]
    fn and_and_or_with_precedence_no_parens() {
        // `a = 1 && b = 2 || c = 3` parses as `(a = 1 && b = 2) || c = 3`
        // — AND binds tighter than OR.
        let node = parse_filter("a = 1 && b = 2 || c = 3").unwrap();
        match node {
            FilterNode::Or(lhs, rhs) => {
                assert!(matches!(*lhs, FilterNode::And(_, _)));
                assert!(matches!(*rhs, FilterNode::Eq(_, _)));
            }
            _ => panic!("expected top-level Or"),
        }
    }

    #[test]
    fn parens_override_precedence() {
        // `a = 1 && (b = 2 || c = 3)` keeps the OR on the inside.
        let node = parse_filter("a = 1 && (b = 2 || c = 3)").unwrap();
        match node {
            FilterNode::And(_, rhs) => assert!(matches!(*rhs, FilterNode::Or(_, _))),
            _ => panic!("expected top-level And"),
        }
    }

    #[test]
    fn not_prefix() {
        let node = parse_filter("!verified = true").unwrap();
        match node {
            FilterNode::Not(inner) => assert!(matches!(*inner, FilterNode::Eq(_, _))),
            _ => panic!("expected Not"),
        }
    }

    #[test]
    fn rejects_empty_input() {
        assert!(parse_filter("").is_err());
        assert!(parse_filter("   ").is_err());
    }

    #[test]
    fn rejects_garbage_after_valid_expr() {
        assert!(parse_filter("a = 1 garbage").is_err());
    }

    #[test]
    fn rejects_unknown_operator() {
        assert!(parse_filter("a === 1").is_err());
    }

    #[test]
    fn rejects_unterminated_string() {
        assert!(parse_filter(r#"a = "hello"#).is_err());
    }
}
