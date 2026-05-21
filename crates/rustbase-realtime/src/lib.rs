//! In-process pub/sub broker for realtime subscriptions.
//!
//! Channels are keyed by `(realm_id, app_id, collection)`. SSE / WS
//! handlers subscribe; record CRUD handlers publish. The broker
//! itself is unaware of HTTP — it just hands out `broadcast::Receiver`s
//! and lets callers turn them into whichever wire format they want.

use dashmap::DashMap;
use rustbase_core::Record;
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::broadcast;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SubscriptionKey {
    pub realm: String,
    pub app: String,
    pub collection: String,
}

impl SubscriptionKey {
    pub fn new(
        realm: impl Into<String>,
        app: impl Into<String>,
        collection: impl Into<String>,
    ) -> Self {
        Self {
            realm: realm.into(),
            app: app.into(),
            collection: collection.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RealtimeEvent {
    RecordCreated { record: Record },
    RecordUpdated { record: Record },
    RecordDeleted { id: String },
}

#[derive(Clone)]
pub struct RealtimeBroker {
    inner: Arc<DashMap<SubscriptionKey, broadcast::Sender<RealtimeEvent>>>,
    /// Per-channel buffer size. Late subscribers tolerate this many
    /// missed events before they get `Lagged`.
    capacity: usize,
}

impl Default for RealtimeBroker {
    fn default() -> Self {
        Self::with_capacity(32)
    }
}

impl RealtimeBroker {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
            capacity: capacity.max(1),
        }
    }

    pub fn subscribe(&self, key: &SubscriptionKey) -> broadcast::Receiver<RealtimeEvent> {
        if let Some(tx) = self.inner.get(key) {
            return tx.subscribe();
        }
        let (tx, rx) = broadcast::channel(self.capacity);
        self.inner.insert(key.clone(), tx);
        rx
    }

    /// Publish an event. Returns the number of subscribers it
    /// reached (0 = none, the event is dropped). If the channel has
    /// no more receivers, it's removed from the map so the broker
    /// doesn't accumulate dead senders.
    pub fn publish(&self, key: &SubscriptionKey, event: RealtimeEvent) -> usize {
        let (delivered, drop_channel) = match self.inner.get(key) {
            Some(tx) => {
                let n = tx.send(event).unwrap_or(0);
                (n, n == 0 && tx.receiver_count() == 0)
            }
            None => (0, false),
        };
        if drop_channel {
            self.inner.remove(key);
        }
        delivered
    }

    pub fn channel_count(&self) -> usize {
        self.inner.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustbase_core::{CollectionId, RecordId};
    use std::collections::BTreeMap;

    fn key() -> SubscriptionKey {
        SubscriptionKey::new("acme", "mobile", "notes")
    }

    fn rec(id: &str) -> Record {
        Record {
            id: RecordId::from(id),
            collection: CollectionId::from("notes"),
            fields: BTreeMap::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn subscriber_receives_published_event() {
        let broker = RealtimeBroker::default();
        let mut rx = broker.subscribe(&key());
        let delivered = broker.publish(
            &key(),
            RealtimeEvent::RecordCreated { record: rec("r1") },
        );
        assert_eq!(delivered, 1);
        let ev = rx.recv().await.unwrap();
        assert!(matches!(ev, RealtimeEvent::RecordCreated { .. }));
    }

    #[tokio::test]
    async fn publish_with_no_subscribers_returns_zero() {
        let broker = RealtimeBroker::default();
        let n = broker.publish(
            &key(),
            RealtimeEvent::RecordDeleted { id: "x".into() },
        );
        assert_eq!(n, 0);
        assert_eq!(broker.channel_count(), 0);
    }

    #[tokio::test]
    async fn channel_removed_after_all_subscribers_drop() {
        let broker = RealtimeBroker::default();
        let rx = broker.subscribe(&key());
        assert_eq!(broker.channel_count(), 1);
        drop(rx);
        broker.publish(
            &key(),
            RealtimeEvent::RecordDeleted { id: "x".into() },
        );
        assert_eq!(broker.channel_count(), 0);
    }

    #[tokio::test]
    async fn distinct_keys_get_distinct_channels() {
        let broker = RealtimeBroker::default();
        let k1 = SubscriptionKey::new("acme", "mobile", "notes");
        let k2 = SubscriptionKey::new("acme", "mobile", "tasks");
        let mut rx1 = broker.subscribe(&k1);
        let mut rx2 = broker.subscribe(&k2);

        broker.publish(&k1, RealtimeEvent::RecordCreated { record: rec("r1") });
        assert!(rx1.recv().await.is_ok());
        assert!(matches!(
            rx2.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));
    }
}
