//! Server-Sent Events subscription endpoint.
//!
//! `GET /api/realms/:realm/apps/:app/collections/:coll/events`
//!
//! Authorisation reuses the records-list rule: a subscriber must
//! pass the same `AccessAction::List` check as `GET .../records`.
//! Each event is a JSON object emitted on the SSE stream; the
//! event kinds match `RealtimeEvent`.

use axum::{
    extract::{Path, State},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
};
use futures::Stream;
use rustbase_db::access_rules::{AccessAction, RuleDecision, classify_rule, get_rule};
use rustbase_realtime::{RealtimeEvent, SubscriptionKey};
use std::convert::Infallible;
use tokio_stream::{StreamExt, wrappers::BroadcastStream};

use crate::auth::PrincipalAuth;
use crate::error::ApiError;
use crate::state::AppState;

pub async fn record_events(
    auth: PrincipalAuth,
    State(state): State<AppState>,
    Path((realm, app, coll)): Path<(String, String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    // Same gate as the records list. Subscribers see exactly what a
    // GET /records would return rows for.
    authorize_subscribe(&auth, &state, &realm, &app, &coll).await?;

    let key = SubscriptionKey::new(&realm, &app, &coll);
    let rx = state.broker.subscribe(&key);

    let stream = BroadcastStream::new(rx).filter_map(|result| match result {
        Ok(ev) => Some(realtime_event_to_sse(ev)),
        // BroadcastStreamRecvError::Lagged: subscriber missed events.
        // Skip silently — clients re-fetch state via GET when this
        // matters. A future branch can surface a "lagged" frame.
        Err(_) => None,
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
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

async fn authorize_subscribe(
    auth: &PrincipalAuth,
    state: &AppState,
    realm: &str,
    app: &str,
    coll: &str,
) -> Result<(), ApiError> {
    use rustbase_core::{AppId, CoreError, RealmId};
    use rustbase_db::{apps::find_app, collections::find_collection, realms::find_realm};

    find_realm(state.system.pool(), realm)
        .await?
        .ok_or(ApiError::Core(CoreError::RealmNotFound(realm.to_string())))?;
    let realm_id = RealmId::from(realm.to_string());
    let realm_pool = state.realms.pool_for(&realm_id).await?;
    find_app(&realm_pool, app).await?.ok_or_else(|| {
        ApiError::Core(CoreError::AppNotFound {
            realm: realm.to_string(),
            app: app.to_string(),
        })
    })?;
    let app_id = AppId::from(app.to_string());
    let app_pool = state.apps.pool_for(&realm_id, &app_id).await?;
    find_collection(&app_pool, coll).await?.ok_or_else(|| {
        ApiError::Core(CoreError::NotFound {
            collection: coll.to_string(),
            id: String::new(),
        })
    })?;

    if auth.is_admin_for_app(realm, app) {
        return Ok(());
    }
    if auth.user_realm() != Some(realm) {
        return Err(ApiError::Core(CoreError::Forbidden));
    }
    let rule = get_rule(&app_pool, coll, AccessAction::List).await?;
    match classify_rule(&rule) {
        RuleDecision::Allow => Ok(()),
        // For SSE, a template rule is currently treated as deny since
        // we don't filter the stream per-record yet. (Per-record SSE
        // gating is a follow-up.)
        _ => Err(ApiError::Core(CoreError::Forbidden)),
    }
}

#[allow(dead_code)]
type SseStream = Box<dyn Stream<Item = Result<Event, Infallible>> + Send + Unpin>;
