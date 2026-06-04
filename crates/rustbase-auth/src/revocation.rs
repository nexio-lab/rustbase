//! In-memory revocation set, keyed by subject.
//!
//! When an admin force-logs-out a user, their `SubjectKey` is added with
//! the current timestamp. Any access token whose `iat` is at or before
//! that timestamp is considered revoked. Entries auto-expire after
//! `access_token_ttl` (because no surviving token can be older than that
//! anyway).

use chrono::Utc;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Identifies a single user / admin across the system.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SubjectKey {
    /// `None` for master admins (which are not scoped to a workspace).
    pub workspace: Option<String>,
    pub subject: String,
}

impl SubjectKey {
    pub fn master(subject: impl Into<String>) -> Self {
        Self {
            workspace: None,
            subject: subject.into(),
        }
    }

    pub fn scoped(workspace: impl Into<String>, subject: impl Into<String>) -> Self {
        Self {
            workspace: Some(workspace.into()),
            subject: subject.into(),
        }
    }
}

#[derive(Clone)]
pub struct RevocationSet {
    inner: Arc<DashMap<SubjectKey, i64>>,
    ttl_seconds: i64,
}

impl RevocationSet {
    pub fn new(ttl_seconds: i64) -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
            ttl_seconds: ttl_seconds.max(0),
        }
    }

    /// Mark `key` as revoked from now. All tokens with `iat <= now` are
    /// invalidated until the entry auto-expires.
    pub fn revoke(&self, key: SubjectKey) {
        self.inner.insert(key, Utc::now().timestamp());
    }

    /// True iff `(key, token_iat)` is invalidated by a current revocation.
    /// Calls `purge_expired` for the matched key as a side effect.
    pub fn is_revoked(&self, key: &SubjectKey, token_iat: i64) -> bool {
        let now = Utc::now().timestamp();
        let snapshot = self.inner.get(key).map(|r| *r.value());
        let revoked_at = match snapshot {
            Some(t) => t,
            None => return false,
        };
        if now > revoked_at + self.ttl_seconds {
            self.inner.remove(key);
            return false;
        }
        token_iat <= revoked_at
    }

    /// Sweep expired entries. Cheap; safe to call from a background task.
    pub fn purge_expired(&self) {
        let cutoff = Utc::now().timestamp() - self.ttl_seconds;
        self.inner.retain(|_, revoked_at| *revoked_at > cutoff);
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

impl Default for RevocationSet {
    /// 15-minute default TTL matches the design's default access-token TTL.
    fn default() -> Self {
        Self::new(15 * 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revoke_invalidates_older_token() {
        let set = RevocationSet::new(900);
        let key = SubjectKey::scoped("acme", "u1");
        let iat_before = Utc::now().timestamp() - 60;
        set.revoke(key.clone());
        assert!(set.is_revoked(&key, iat_before));
    }

    #[test]
    fn revoke_does_not_invalidate_newer_token() {
        let set = RevocationSet::new(900);
        let key = SubjectKey::scoped("acme", "u1");
        set.revoke(key.clone());
        let iat_after = Utc::now().timestamp() + 60;
        assert!(!set.is_revoked(&key, iat_after));
    }

    #[test]
    fn unrelated_subjects_are_not_revoked() {
        let set = RevocationSet::new(900);
        set.revoke(SubjectKey::scoped("acme", "u1"));
        assert!(!set.is_revoked(&SubjectKey::scoped("acme", "u2"), 0));
        assert!(!set.is_revoked(&SubjectKey::scoped("widgetco", "u1"), 0));
    }

    #[test]
    fn master_subject_key_has_no_realm() {
        let set = RevocationSet::new(900);
        let key = SubjectKey::master("admin-1");
        set.revoke(key.clone());
        assert!(set.is_revoked(&key, Utc::now().timestamp() - 10));
    }

    #[test]
    fn purge_drops_expired_entries() {
        let set = RevocationSet::new(0); // immediate expiry
        let key = SubjectKey::scoped("acme", "u1");
        set.revoke(key.clone());
        std::thread::sleep(std::time::Duration::from_millis(1100));
        set.purge_expired();
        assert!(set.is_empty());
    }
}
