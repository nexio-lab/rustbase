//! Realtime subscription endpoints.
//!
//! Two transports against the same in-process broker:
//!
//! - **SSE** (`GET /api/workspaces/:workspace/apps/:app/collections/:coll/events`)
//!   — one-way text stream. The default for browser clients that want
//!   live record updates without writing back.
//! - **WebSocket** (`GET …/collections/:coll/events/ws`) — a thin
//!   wrapper around the same broker for clients that prefer a single
//!   long-lived connection (mobile, custom SDKs). Server pushes
//!   identical JSON event frames; the client never sends data after
//!   the upgrade.
//!
//! Both endpoints accept an optional `?filter=<expression>` query
//! parameter. When set, only events whose record matches the filter
//! reach the subscriber — the same `FilterNode` AST that
//! `GET .../records` uses, evaluated in-memory against the record's
//! fields. Delete events have no record body and bypass the filter
//! (subscribers always get deletes so they can drop the row from
//! their cache).
//!
//! Authorisation reuses the records-list access rule. Template
//! rules (e.g. "owner = @request.auth.id") are now supported on
//! realtime: the template is materialised against the principal and
//! intersected with any client-supplied filter.

use axum::{
    extract::{
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures::Stream;
use rustbase_core::{CoreError, FilterNode, RuleContext, parse_filter, substitute_rule_template};
use rustbase_db::access_rules::{AccessAction, RuleDecision, classify_rule, get_rule};
use rustbase_realtime::{RealtimeEvent, SubscriptionKey};
use serde::Deserialize;
use std::convert::Infallible;
use tokio_stream::{StreamExt, wrappers::BroadcastStream};

use crate::auth::{PrincipalAuth, principal_from_token};
use crate::error::ApiError;
use crate::state::AppState;

/// Optional filter expression on either transport.
#[derive(Debug, Deserialize, Default)]
pub struct EventsQuery {
    #[serde(default)]
    pub filter: Option<String>,
}

/// WS-only query: same `filter` as SSE, plus an optional access token
/// fallback. The browser `WebSocket` constructor can't set
/// `Authorization: Bearer ...`, so the client SDK passes it as
/// `?token=<jwt>` instead — the handler honours it only after the
/// header + cookie paths come back empty. Tokens-in-URLs end up in
/// access logs; the SDK avoids the leak in regular fetch paths.
#[derive(Debug, Deserialize, Default)]
pub struct EventsQueryWs {
    #[serde(default)]
    pub filter: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
}

pub async fn record_events(
    auth: PrincipalAuth,
    State(state): State<AppState>,
    Path((workspace, app, coll)): Path<(String, String, String)>,
    Query(q): Query<EventsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let filter =
        authorize_subscribe(&auth, &state, &workspace, &app, &coll, q.filter.as_deref()).await?;

    let key = SubscriptionKey::new(&workspace, &app, &coll);
    let rx = state.broker.subscribe(&key);

    let stream = BroadcastStream::new(rx).filter_map(move |result| match result {
        Ok(ev) if event_matches(&ev, filter.as_ref()) => Some(realtime_event_to_sse(ev)),
        Ok(_) => None,
        // BroadcastStreamRecvError::Lagged: subscriber missed events.
        // Skip silently — clients re-fetch state via GET when this
        // matters.
        Err(_) => None,
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// `GET …/events/ws`. Same filter shape + the SSE endpoint;
/// the protocol after upgrade is one server→client JSON frame per
/// event, matching the SSE `data` payload byte-for-byte.
///
/// Auth resolution differs from SSE: header → cookie → `?token=`
/// query. The query fallback exists because the browser
/// `WebSocket` constructor can't set request headers, so the SDK
/// passes the token in the URL on connect.
pub async fn record_events_ws(
    State(state): State<AppState>,
    Path((workspace, app, coll)): Path<(String, String, String)>,
    Query(q): Query<EventsQueryWs>,
    headers: axum::http::HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<axum::response::Response, ApiError> {
    let token = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer ").map(str::to_string))
        .or_else(|| {
            crate::auth::cookies::read_cookie(&headers, crate::auth::cookies::ACCESS_COOKIE)
        })
        .or(q.token);
    let auth = match token {
        Some(t) => principal_from_token(&state, &t)?,
        None => return Err(ApiError::Core(CoreError::Unauthorized)),
    };
    let filter =
        authorize_subscribe(&auth, &state, &workspace, &app, &coll, q.filter.as_deref()).await?;
    let key = SubscriptionKey::new(&workspace, &app, &coll);
    let rx = state.broker.subscribe(&key);
    Ok(ws.on_upgrade(move |socket| ws_pump(socket, rx, filter)))
}

async fn ws_pump(
    mut socket: WebSocket,
    rx: tokio::sync::broadcast::Receiver<RealtimeEvent>,
    filter: Option<FilterNode>,
) {
    let mut stream = BroadcastStream::new(rx);
    loop {
        tokio::select! {
            // Forward broker events that match the filter.
            ev = stream.next() => {
                match ev {
                    Some(Ok(ev)) if event_matches(&ev, filter.as_ref()) => {
                        let payload = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into());
                        if socket.send(Message::Text(payload.into())).await.is_err() {
                            return;
                        }
                    }
                    Some(Ok(_)) => continue,
                    Some(Err(_)) => continue, // lagged — skip
                    None => return,           // broker closed
                }
            }
            // Drain client frames so axum's ping/pong handling can
            // detect disconnects. We don't interpret anything the
            // client sends; the API is push-only.
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => return,
                    Some(Err(_)) => return,
                    Some(Ok(_)) => continue,
                }
            }
        }
    }
}

fn event_matches(ev: &RealtimeEvent, filter: Option<&FilterNode>) -> bool {
    let Some(f) = filter else { return true };
    match ev {
        RealtimeEvent::RecordCreated { record } | RealtimeEvent::RecordUpdated { record } => {
            f.matches(&record.fields)
        }
        // Delete events carry no record body. Push them through
        // unconditionally so subscribers can evict cached rows even
        // when the row never matched the filter — defensive against
        // a row whose values changed *out of* the filter set being
        // missed by the client.
        RealtimeEvent::RecordDeleted { .. } => true,
    }
}

fn realtime_event_to_sse(ev: RealtimeEvent) -> Result<Event, Infallible> {
    let kind = match &ev {
        RealtimeEvent::RecordCreated { .. } => "record_created",
        RealtimeEvent::RecordUpdated { .. } => "record_updated",
        RealtimeEvent::RecordDeleted { .. } => "record_deleted",
    };
    let payload = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into());
    Ok(Event::default().event(kind).data(payload))
}

/// Resolve the effective server-side filter for this subscription:
/// the intersection of the collection's access rule (after template
/// substitution) and the optional `?filter=` from the client. Also
/// performs the workspace / app / collection existence checks.
///
/// Returns `Ok(None)` when no filter applies (admin path or
/// `RuleDecision::Allow` + no client filter), or `Ok(Some(node))`
/// with the combined AST otherwise. Returns `Forbidden` when the
/// principal can't subscribe at all.
async fn authorize_subscribe(
    auth: &PrincipalAuth,
    state: &AppState,
    workspace: &str,
    app: &str,
    coll: &str,
    client_filter: Option<&str>,
) -> Result<Option<FilterNode>, ApiError> {
    use rustbase_core::{AppId, WorkspaceId};
    use rustbase_db::{apps::find_app, collections::find_collection, workspaces::find_workspace};

    find_workspace(state.system.pool(), workspace)
        .await?
        .ok_or(ApiError::Core(CoreError::WorkspaceNotFound(
            workspace.to_string(),
        )))?;
    let workspace_id = WorkspaceId::from(workspace.to_string());
    let workspace_pool = state.workspaces.pool_for(&workspace_id).await?;
    find_app(&workspace_pool, app).await?.ok_or_else(|| {
        ApiError::Core(CoreError::AppNotFound {
            workspace: workspace.to_string(),
            app: app.to_string(),
        })
    })?;
    let app_id = AppId::from(app.to_string());
    let app_pool = state.apps.pool_for(&workspace_id, &app_id).await?;
    find_collection(&app_pool, coll).await?.ok_or_else(|| {
        ApiError::Core(CoreError::NotFound {
            collection: coll.to_string(),
            id: String::new(),
        })
    })?;

    let client_filter = client_filter
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| -> Result<FilterNode, ApiError> {
            parse_filter(s)
                .map_err(|e| ApiError::Core(CoreError::Validation(format!("filter: {e}"))))
        })
        .transpose()?;

    if auth.is_admin_for_app(workspace, app) {
        return Ok(client_filter);
    }
    if auth.user_workspace() != Some(workspace) {
        return Err(ApiError::Core(CoreError::Forbidden));
    }

    let rule = get_rule(&app_pool, coll, AccessAction::List).await?;
    let rule_filter: Option<FilterNode> = match classify_rule(&rule) {
        RuleDecision::Deny => return Err(ApiError::Core(CoreError::Forbidden)),
        RuleDecision::Allow => None,
        RuleDecision::Evaluate(template) => {
            // Substitute the principal into the template, then parse.
            // Same materialisation path the records handler takes.
            let ctx = RuleContext {
                user_id: Some(auth.subject_id.clone()),
                user_email: None,
                user_workspace: auth.user_workspace().map(str::to_string),
            };
            let materialised = substitute_rule_template(&template, &ctx).map_err(|e| {
                ApiError::Core(CoreError::Internal(format!(
                    "rule template substitute: {e}"
                )))
            })?;
            Some(parse_filter(&materialised).map_err(|e| {
                ApiError::Core(CoreError::Internal(format!("rule template parse: {e}")))
            })?)
        }
    };

    Ok(intersect_filters(rule_filter, client_filter))
}

fn intersect_filters(a: Option<FilterNode>, b: Option<FilterNode>) -> Option<FilterNode> {
    match (a, b) {
        (None, None) => None,
        (Some(x), None) | (None, Some(x)) => Some(x),
        (Some(x), Some(y)) => Some(FilterNode::and(x, y)),
    }
}

#[allow(dead_code)]
type SseStream = Box<dyn Stream<Item = Result<Event, Infallible>> + Send + Unpin>;

#[cfg(test)]
mod tests {
    use super::*;
    use rustbase_core::Record;
    use rustbase_core::{CollectionId, RecordId};
    use serde_json::json;
    use std::collections::BTreeMap;

    fn rec(fields: &[(&str, serde_json::Value)]) -> Record {
        let map: BTreeMap<String, serde_json::Value> = fields
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        Record {
            id: RecordId::from("r1"),
            collection: CollectionId::from("notes"),
            fields: map,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn event_matches_no_filter_lets_everything_through() {
        let ev = RealtimeEvent::RecordCreated {
            record: rec(&[("status", json!("open"))]),
        };
        assert!(event_matches(&ev, None));
    }

    #[test]
    fn event_matches_filter_on_created_and_updated() {
        let f = FilterNode::Eq("status".into(), json!("open"));
        let created = RealtimeEvent::RecordCreated {
            record: rec(&[("status", json!("open"))]),
        };
        let updated_match = RealtimeEvent::RecordUpdated {
            record: rec(&[("status", json!("open"))]),
        };
        let updated_drop = RealtimeEvent::RecordUpdated {
            record: rec(&[("status", json!("closed"))]),
        };
        assert!(event_matches(&created, Some(&f)));
        assert!(event_matches(&updated_match, Some(&f)));
        assert!(!event_matches(&updated_drop, Some(&f)));
    }

    #[test]
    fn event_matches_lets_deletes_through_regardless() {
        let f = FilterNode::Eq("status".into(), json!("open"));
        let del = RealtimeEvent::RecordDeleted { id: "r-99".into() };
        assert!(event_matches(&del, Some(&f)));
    }

    #[test]
    fn intersect_filters_and_combines_both() {
        let a = FilterNode::Eq("status".into(), json!("open"));
        let b = FilterNode::Gt("age".into(), json!(17));
        let combined = intersect_filters(Some(a.clone()), Some(b.clone())).unwrap();
        match combined {
            FilterNode::And(l, r) => {
                assert_eq!(*l, a);
                assert_eq!(*r, b);
            }
            _ => panic!("expected And"),
        }
    }

    #[test]
    fn intersect_filters_empty_sides_pass_through() {
        let a = FilterNode::Eq("k".into(), json!(1));
        assert_eq!(intersect_filters(Some(a.clone()), None), Some(a.clone()));
        assert_eq!(intersect_filters(None, Some(a.clone())), Some(a));
        assert_eq!(intersect_filters(None, None), None);
    }
}
