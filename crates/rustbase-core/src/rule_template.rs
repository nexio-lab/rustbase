//! Pre-parse template substitution for access rules.
//!
//! A stored rule may reference request-context values with the
//! `{{path}}` syntax:
//!
//! ```text
//! owner = {{request.auth.id}}
//! ```
//!
//! `substitute()` replaces every `{{path}}` with the JSON-encoded
//! value from `RuleContext`. The resulting string is then handed to
//! `parse_filter`, so the rest of the filter pipeline (validation,
//! SQL translation) stays exactly as it is.
//!
//! Supported paths today:
//!   - `request.auth.id`
//!   - `request.auth.email`
//!   - `request.auth.realm`
//!
//! Unknown paths return an error; this is by design — silently
//! emitting `null` would invite rules that look strict but are
//! actually open.

use crate::error::CoreError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuleContext {
    pub user_id: Option<String>,
    pub user_email: Option<String>,
    pub user_realm: Option<String>,
}

pub fn substitute(template: &str, ctx: &RuleContext) -> Result<String, CoreError> {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            // find closing }}
            let start = i + 2;
            let mut j = start;
            while j + 1 < bytes.len() && !(bytes[j] == b'}' && bytes[j + 1] == b'}') {
                j += 1;
            }
            if j + 1 >= bytes.len() || bytes[j] != b'}' || bytes[j + 1] != b'}' {
                return Err(CoreError::Validation(format!(
                    "unterminated {{{{ }}}} in rule template"
                )));
            }
            let path = std::str::from_utf8(&bytes[start..j])
                .map_err(|e| CoreError::Validation(format!("invalid utf-8 in template: {e}")))?
                .trim();
            out.push_str(&resolve(path, ctx)?);
            i = j + 2;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    Ok(out)
}

fn resolve(path: &str, ctx: &RuleContext) -> Result<String, CoreError> {
    let value: Option<&str> = match path {
        "request.auth.id" => ctx.user_id.as_deref(),
        "request.auth.email" => ctx.user_email.as_deref(),
        "request.auth.realm" => ctx.user_realm.as_deref(),
        other => {
            return Err(CoreError::Validation(format!(
                "unknown rule-template path: {other}"
            )));
        }
    };
    Ok(match value {
        Some(v) => serde_json::to_string(v).unwrap_or_else(|_| "null".into()),
        None => "null".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(id: &str) -> RuleContext {
        RuleContext {
            user_id: Some(id.into()),
            user_email: Some("u@example.com".into()),
            user_realm: Some("acme".into()),
        }
    }

    #[test]
    fn substitutes_simple_id() {
        let out = substitute("owner = {{request.auth.id}}", &ctx("u123")).unwrap();
        assert_eq!(out, r#"owner = "u123""#);
    }

    #[test]
    fn substitutes_email_and_realm() {
        let out = substitute(
            "domain = {{request.auth.email}} && realm = {{request.auth.realm}}",
            &ctx("u1"),
        )
        .unwrap();
        assert_eq!(out, r#"domain = "u@example.com" && realm = "acme""#);
    }

    #[test]
    fn escapes_double_quotes_in_value() {
        let mut c = ctx("u1");
        c.user_email = Some(r#""weird"@x.com"#.into());
        let out = substitute("email = {{request.auth.email}}", &c).unwrap();
        // serde_json::to_string escapes embedded quotes
        assert_eq!(out, r#"email = "\"weird\"@x.com""#);
    }

    #[test]
    fn unknown_path_is_an_error() {
        let err = substitute("x = {{request.fictional}}", &ctx("u1")).unwrap_err();
        assert!(matches!(err, CoreError::Validation(_)));
    }

    #[test]
    fn unterminated_braces_are_an_error() {
        let err = substitute("owner = {{request.auth.id", &ctx("u1")).unwrap_err();
        assert!(matches!(err, CoreError::Validation(_)));
    }

    #[test]
    fn no_braces_returns_input_unchanged() {
        let out = substitute(r#"role = "admin""#, &ctx("u1")).unwrap();
        assert_eq!(out, r#"role = "admin""#);
    }

    #[test]
    fn missing_context_value_becomes_null() {
        let c = RuleContext::default();
        let out = substitute("uid = {{request.auth.id}}", &c).unwrap();
        assert_eq!(out, "uid = null");
    }

    #[test]
    fn trims_whitespace_inside_braces() {
        let out = substitute("uid = {{ request.auth.id }}", &ctx("u1")).unwrap();
        assert_eq!(out, r#"uid = "u1""#);
    }
}
