//! Hierarchical configuration policy primitives.
//!
//! Each configurable knob (password length, token TTL, allowed OAuth
//! providers, etc.) is represented as a `PolicySpec`. Master, realm, and app
//! each hold their own `PolicySpec` for the knob; validation walks parent →
//! child and rejects values outside the parent's bound. When a parent
//! tightens its bound, the same primitives are used to clamp the child.
//!
//! The actual policy fields (which knob exists, what kind it is) are
//! declared in higher layers; this module only provides the mechanism.

use crate::error::{CoreError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Top-level policy variant. A given knob always has the same variant at
/// every level — mixing kinds across levels is a programmer error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PolicySpec {
    Range(RangePolicy),
    Toggle(TogglePolicy),
    EnumSet(EnumSetPolicy),
    Free(serde_json::Value),
}

impl PolicySpec {
    /// True iff `child` is a valid refinement of `self`.
    pub fn allows(&self, child: &PolicySpec) -> bool {
        match (self, child) {
            (PolicySpec::Range(p), PolicySpec::Range(c)) => p.allows(c),
            (PolicySpec::Toggle(p), PolicySpec::Toggle(c)) => p.allows(c),
            (PolicySpec::EnumSet(p), PolicySpec::EnumSet(c)) => p.allows(c),
            (PolicySpec::Free(_), PolicySpec::Free(_)) => true,
            _ => false,
        }
    }

    /// Clamp `child` into the bounds of `self`, returning a refinement that
    /// satisfies `self.allows(...)`. Kind mismatches return the child
    /// unchanged — they should be caught by the validator before this is
    /// called.
    pub fn clamp(&self, child: PolicySpec) -> PolicySpec {
        match (self, child) {
            (PolicySpec::Range(p), PolicySpec::Range(c)) => PolicySpec::Range(p.clamp(c)),
            (PolicySpec::Toggle(p), PolicySpec::Toggle(c)) => PolicySpec::Toggle(p.clamp(c)),
            (PolicySpec::EnumSet(p), PolicySpec::EnumSet(c)) => PolicySpec::EnumSet(p.clamp(c)),
            (_, child) => child,
        }
    }

    /// Validate `child` against `self`, returning `PolicyViolation` if rejected.
    pub fn validate(&self, field: &str, child: &PolicySpec) -> Result<()> {
        if self.allows(child) {
            Ok(())
        } else {
            Err(CoreError::PolicyViolation {
                field: field.to_string(),
                value: format!("{child:?}"),
                bound: format!("{self:?}"),
            })
        }
    }
}

/// Inclusive numeric range. Child range must be a subset of parent.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RangePolicy {
    pub min: i64,
    pub max: i64,
}

impl RangePolicy {
    pub fn new(min: i64, max: i64) -> Result<Self> {
        if min > max {
            return Err(CoreError::Validation(format!(
                "range min {min} > max {max}"
            )));
        }
        Ok(Self { min, max })
    }

    pub fn allows(&self, child: &RangePolicy) -> bool {
        child.min >= self.min && child.max <= self.max && child.min <= child.max
    }

    /// Clamp `child` into `self`. If the intersection is empty, fall back
    /// to the parent's bounds.
    pub fn clamp(&self, child: RangePolicy) -> RangePolicy {
        let min = child.min.max(self.min);
        let max = child.max.min(self.max);
        if min > max {
            *self
        } else {
            RangePolicy { min, max }
        }
    }
}

/// Boolean toggle with an optional lock from the parent.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TogglePolicy {
    /// Children may pick freely; the default is provided.
    Open { default: bool },
    /// Children must use this value.
    Locked { value: bool },
}

impl TogglePolicy {
    pub fn allows(&self, child: &TogglePolicy) -> bool {
        match (self, child) {
            (TogglePolicy::Locked { value: p }, TogglePolicy::Locked { value: c }) => p == c,
            (TogglePolicy::Locked { value: p }, TogglePolicy::Open { default: c }) => p == c,
            (TogglePolicy::Open { .. }, _) => true,
        }
    }

    pub fn clamp(&self, child: TogglePolicy) -> TogglePolicy {
        match self {
            TogglePolicy::Locked { value } => TogglePolicy::Locked { value: *value },
            TogglePolicy::Open { .. } => child,
        }
    }

    /// The currently effective value once you take the lock into account.
    pub fn effective(&self) -> bool {
        match self {
            TogglePolicy::Open { default } => *default,
            TogglePolicy::Locked { value } => *value,
        }
    }
}

/// Whitelist of allowed strings (e.g. OAuth provider names, MIME types).
/// Child set must be a subset of parent set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnumSetPolicy {
    pub allowed: BTreeSet<String>,
}

impl EnumSetPolicy {
    pub fn new<I, S>(values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            allowed: values.into_iter().map(Into::into).collect(),
        }
    }

    pub fn allows(&self, child: &EnumSetPolicy) -> bool {
        child.allowed.is_subset(&self.allowed)
    }

    pub fn clamp(&self, child: EnumSetPolicy) -> EnumSetPolicy {
        EnumSetPolicy {
            allowed: child
                .allowed
                .intersection(&self.allowed)
                .cloned()
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_allows_subset_rejects_superset() {
        let master = RangePolicy::new(4, 64).unwrap();
        let realm_inside = RangePolicy::new(8, 32).unwrap();
        let realm_outside = RangePolicy::new(2, 100).unwrap();
        assert!(master.allows(&realm_inside));
        assert!(!master.allows(&realm_outside));
    }

    #[test]
    fn range_clamps_to_parent_bounds() {
        let master = RangePolicy::new(4, 64).unwrap();
        let realm = RangePolicy::new(2, 100).unwrap();
        let clamped = master.clamp(realm);
        assert_eq!(clamped, RangePolicy { min: 4, max: 64 });
    }

    #[test]
    fn range_clamp_with_no_overlap_falls_back_to_parent() {
        let master = RangePolicy::new(10, 20).unwrap();
        let stale = RangePolicy::new(100, 200).unwrap();
        assert_eq!(master.clamp(stale), master);
    }

    #[test]
    fn toggle_open_parent_allows_anything() {
        let parent = TogglePolicy::Open { default: false };
        assert!(parent.allows(&TogglePolicy::Locked { value: true }));
        assert!(parent.allows(&TogglePolicy::Open { default: true }));
    }

    #[test]
    fn toggle_locked_parent_forces_value() {
        let parent = TogglePolicy::Locked { value: true };
        assert!(parent.allows(&TogglePolicy::Locked { value: true }));
        assert!(!parent.allows(&TogglePolicy::Locked { value: false }));
        assert!(parent.allows(&TogglePolicy::Open { default: true }));
        assert!(!parent.allows(&TogglePolicy::Open { default: false }));
    }

    #[test]
    fn toggle_clamp_to_locked() {
        let parent = TogglePolicy::Locked { value: true };
        let child = TogglePolicy::Open { default: false };
        assert_eq!(parent.clamp(child), TogglePolicy::Locked { value: true });
    }

    #[test]
    fn enum_set_subset_passes_superset_fails() {
        let master = EnumSetPolicy::new(["google", "github", "email"]);
        let realm_ok = EnumSetPolicy::new(["google", "email"]);
        let realm_bad = EnumSetPolicy::new(["google", "facebook"]);
        assert!(master.allows(&realm_ok));
        assert!(!master.allows(&realm_bad));
    }

    #[test]
    fn enum_set_clamp_intersects() {
        let master = EnumSetPolicy::new(["google", "github", "email"]);
        let realm = EnumSetPolicy::new(["google", "facebook"]);
        let clamped = master.clamp(realm);
        assert_eq!(clamped.allowed, ["google".to_string()].into_iter().collect());
    }

    #[test]
    fn policy_spec_validate_returns_violation() {
        let master = PolicySpec::Range(RangePolicy::new(4, 64).unwrap());
        let bad = PolicySpec::Range(RangePolicy::new(2, 100).unwrap());
        let err = master.validate("password.length", &bad).unwrap_err();
        assert!(matches!(err, CoreError::PolicyViolation { .. }));
    }

    #[test]
    fn policy_spec_kind_mismatch_rejects() {
        let master = PolicySpec::Toggle(TogglePolicy::Open { default: true });
        let weird = PolicySpec::Range(RangePolicy::new(1, 2).unwrap());
        assert!(!master.allows(&weird));
    }

    #[test]
    fn policy_spec_round_trips_through_json() {
        let spec = PolicySpec::Range(RangePolicy::new(4, 64).unwrap());
        let json = serde_json::to_string(&spec).unwrap();
        let parsed: PolicySpec = serde_json::from_str(&json).unwrap();
        assert_eq!(spec, parsed);
    }
}
