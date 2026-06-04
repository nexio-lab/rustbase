//! Per-subject failed-login tracker — the application-layer counterpart
//! to the IP-based rate limiter that lives in middleware.
//!
//! The IP layer protects against scripted floods regardless of credentials.
//! This layer protects an *individual identity* against credential-stuffing
//! and password-spray when an attacker rotates source IPs. Failed login
//! attempts accumulate against a stable subject key (e.g. `master:admin`,
//! `workspace:acme:user:alice@x.tld`); past the configured threshold inside
//! the rolling window, further attempts are short-circuited with
//! `CoreError::TooManyRequests` until the lockout expires.
//!
//! All state is in-process — single-instance deployments only, which is
//! the design (see the deployment guide).

use chrono::{DateTime, Duration, Utc};
use dashmap::DashMap;
use rustbase_core::CoreError;
use std::sync::Arc;

/// Configuration knobs for the per-subject failed-login tracker.
///
/// Mirrors `LockoutConfig` in `rustbase-server::config` — kept here so
/// the API crate stays unaware of the server crate.
#[derive(Debug, Clone, Copy)]
pub struct LockoutPolicy {
    pub enabled: bool,
    pub max_failures: u32,
    pub window: Duration,
    pub lockout: Duration,
}

impl LockoutPolicy {
    pub fn from_secs(
        enabled: bool,
        max_failures: u32,
        window_secs: u64,
        lockout_secs: u64,
    ) -> Self {
        Self {
            enabled,
            max_failures,
            window: Duration::seconds(window_secs.min(i64::MAX as u64) as i64),
            lockout: Duration::seconds(lockout_secs.min(i64::MAX as u64) as i64),
        }
    }
}

impl Default for LockoutPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            max_failures: 5,
            window: Duration::seconds(300),
            lockout: Duration::seconds(300),
        }
    }
}

#[derive(Debug, Clone)]
struct AttemptState {
    /// Monotonic-ish times of each failure inside the current window.
    /// Pruned on every observation.
    failures: Vec<DateTime<Utc>>,
    /// Set when a lockout fires. Cleared on the first successful login
    /// or on natural expiry.
    locked_until: Option<DateTime<Utc>>,
}

impl AttemptState {
    fn new() -> Self {
        Self {
            failures: Vec::new(),
            locked_until: None,
        }
    }
}

/// In-process map of `subject → AttemptState`, cloneable cheaply.
#[derive(Clone, Default, Debug)]
pub struct LoginAttempts {
    inner: Arc<DashMap<String, AttemptState>>,
}

impl LoginAttempts {
    pub fn new() -> Self {
        Self::default()
    }

    /// Call before consulting the credential store. Returns
    /// `TooManyRequests` if the subject is currently locked; the wrapped
    /// `retry_after_secs` is suitable as the HTTP `Retry-After` value.
    pub fn check(&self, subject: &str, policy: &LockoutPolicy) -> Result<(), CoreError> {
        if !policy.enabled {
            return Ok(());
        }
        let Some(state) = self.inner.get(subject) else {
            return Ok(());
        };
        let Some(until) = state.locked_until else {
            return Ok(());
        };
        let now = Utc::now();
        if until > now {
            let remaining = (until - now).num_seconds().max(1) as u64;
            return Err(CoreError::TooManyRequests {
                retry_after_secs: remaining,
            });
        }
        Ok(())
    }

    /// Record a credential failure. If this crosses `max_failures` inside
    /// `window`, mark the subject as locked for `lockout` and return
    /// `TooManyRequests`; otherwise return `Ok(())` so the caller keeps
    /// returning its usual `Unauthorized` to the client.
    pub fn note_failure(&self, subject: &str, policy: &LockoutPolicy) -> Result<(), CoreError> {
        if !policy.enabled {
            return Ok(());
        }
        let now = Utc::now();
        let window_start = now - policy.window;
        let mut entry = self
            .inner
            .entry(subject.to_string())
            .or_insert_with(AttemptState::new);

        // If a prior lockout has not yet expired, just bubble it back.
        if let Some(until) = entry.locked_until {
            if until > now {
                let remaining = (until - now).num_seconds().max(1) as u64;
                return Err(CoreError::TooManyRequests {
                    retry_after_secs: remaining,
                });
            }
            entry.locked_until = None;
            entry.failures.clear();
        }

        entry.failures.retain(|t| *t > window_start);
        entry.failures.push(now);

        if entry.failures.len() as u32 >= policy.max_failures {
            entry.locked_until = Some(now + policy.lockout);
            let remaining = policy.lockout.num_seconds().max(1) as u64;
            return Err(CoreError::TooManyRequests {
                retry_after_secs: remaining,
            });
        }
        Ok(())
    }

    /// Clear all failure history for a subject — call after a verified
    /// login so the next failure series starts from a clean slate.
    pub fn note_success(&self, subject: &str) {
        self.inner.remove(subject);
    }

    /// Test-only: peek at the current failure count.
    #[cfg(test)]
    pub(crate) fn failure_count(&self, subject: &str) -> usize {
        self.inner
            .get(subject)
            .map(|e| e.failures.len())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> LockoutPolicy {
        LockoutPolicy::from_secs(true, 3, 60, 60)
    }

    #[test]
    fn check_passes_when_disabled() {
        let attempts = LoginAttempts::new();
        let disabled = LockoutPolicy::from_secs(false, 1, 1, 1);
        for _ in 0..10 {
            let _ = attempts.note_failure("alice", &disabled);
        }
        assert!(attempts.check("alice", &disabled).is_ok());
    }

    #[test]
    fn note_failure_locks_at_threshold() {
        let attempts = LoginAttempts::new();
        let p = policy();
        assert!(attempts.note_failure("alice", &p).is_ok());
        assert!(attempts.note_failure("alice", &p).is_ok());
        let err = attempts.note_failure("alice", &p).unwrap_err();
        match err {
            CoreError::TooManyRequests { retry_after_secs } => {
                assert!(retry_after_secs > 0);
            }
            other => panic!("expected TooManyRequests, got {other:?}"),
        }
        assert!(attempts.check("alice", &p).is_err());
    }

    #[test]
    fn note_success_clears_state() {
        let attempts = LoginAttempts::new();
        let p = policy();
        let _ = attempts.note_failure("alice", &p);
        let _ = attempts.note_failure("alice", &p);
        assert_eq!(attempts.failure_count("alice"), 2);
        attempts.note_success("alice");
        assert_eq!(attempts.failure_count("alice"), 0);
        assert!(attempts.check("alice", &p).is_ok());
    }

    #[test]
    fn subjects_are_independent() {
        let attempts = LoginAttempts::new();
        let p = policy();
        let _ = attempts.note_failure("alice", &p);
        let _ = attempts.note_failure("alice", &p);
        let _ = attempts.note_failure("alice", &p);
        assert!(attempts.check("alice", &p).is_err());
        assert!(attempts.check("bob", &p).is_ok());
    }
}
