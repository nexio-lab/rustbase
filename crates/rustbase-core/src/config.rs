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

/// One position in a policy chain. Names are surfaced in audit entries
/// and in `PolicyViolation` errors so the operator can see which level
/// (master / realm / app / …) actually owns a value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyLevel {
    /// Human label for this level (e.g. `"master"`, `"realm"`, `"app"`).
    pub level: String,
    /// The policy at this level.
    pub spec: PolicySpec,
}

impl PolicyLevel {
    pub fn new(level: impl Into<String>, spec: PolicySpec) -> Self {
        Self {
            level: level.into(),
            spec,
        }
    }
}

/// A single change emitted when the cascade rewrites a level to fit a
/// tightened parent. The DB layer consumes these to populate the audit
/// log on master/realm tighten operations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyChange {
    pub field: String,
    pub level: String,
    pub before: PolicySpec,
    pub after: PolicySpec,
}

/// Validate a top-down chain of policies for `field`. Each successive
/// level must be a refinement of the level above it. Returns the
/// `PolicyViolation` for the first offending pair, naming the child
/// level so the operator knows where the broken value lives.
///
/// Levels are validated pairwise — *grand-parent → parent → child*. A
/// transitive failure (child fits parent but parent doesn't fit
/// grand-parent) still trips, because the cascade is broken regardless
/// of whether the leaf itself happens to align with the root.
pub fn validate_chain(field: &str, levels: &[PolicyLevel]) -> Result<()> {
    for win in levels.windows(2) {
        let parent = &win[0];
        let child = &win[1];
        if !parent.spec.allows(&child.spec) {
            return Err(CoreError::PolicyViolation {
                field: format!("{field} ({})", child.level),
                value: format!("{:?}", child.spec),
                bound: format!("{:?} (from {})", parent.spec, parent.level),
            });
        }
    }
    Ok(())
}

/// Cascade a master tightening down through a chain. Given the new
/// top-level spec and the current `levels[1..]` (realm, app, …), walk
/// top-down and clamp each level against the cumulative parent. Returns
/// the rebuilt chain plus a list of changes for the audit log.
///
/// The caller passes `levels` with the *new* top-level spec already in
/// position 0 — typically because they just edited it. This function
/// rewrites positions 1..n so the whole chain re-validates.
pub fn cascade_clamp(field: &str, levels: Vec<PolicyLevel>) -> (Vec<PolicyLevel>, Vec<PolicyChange>) {
    let mut changes = Vec::new();
    let mut out: Vec<PolicyLevel> = Vec::with_capacity(levels.len());

    for (idx, level) in levels.into_iter().enumerate() {
        if idx == 0 {
            out.push(level);
            continue;
        }
        let parent_spec = out[idx - 1].spec.clone();
        if parent_spec.allows(&level.spec) {
            out.push(level);
        } else {
            let before = level.spec.clone();
            let after = parent_spec.clamp(level.spec);
            changes.push(PolicyChange {
                field: field.to_string(),
                level: level.level.clone(),
                before,
                after: after.clone(),
            });
            out.push(PolicyLevel {
                level: level.level,
                spec: after,
            });
        }
    }

    (out, changes)
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

    // ---- chain validator ----

    fn r(min: i64, max: i64) -> PolicySpec {
        PolicySpec::Range(RangePolicy::new(min, max).unwrap())
    }

    #[test]
    fn validate_chain_accepts_nested_refinements() {
        let chain = vec![
            PolicyLevel::new("master", r(4, 64)),
            PolicyLevel::new("realm", r(8, 32)),
            PolicyLevel::new("app", r(10, 16)),
        ];
        validate_chain("password.length", &chain).unwrap();
    }

    #[test]
    fn validate_chain_rejects_app_outside_realm() {
        let chain = vec![
            PolicyLevel::new("master", r(4, 64)),
            PolicyLevel::new("realm", r(8, 32)),
            PolicyLevel::new("app", r(2, 100)),
        ];
        let err = validate_chain("password.length", &chain).unwrap_err();
        match err {
            CoreError::PolicyViolation { field, .. } => {
                assert!(field.contains("password.length"), "got {field}");
                assert!(field.contains("app"), "blame should point at the leaf level: {field}");
            }
            other => panic!("expected PolicyViolation, got: {other:?}"),
        }
    }

    #[test]
    fn validate_chain_rejects_realm_outside_master_even_if_app_inside_realm() {
        let chain = vec![
            PolicyLevel::new("master", r(10, 20)),
            PolicyLevel::new("realm", r(2, 30)),
            PolicyLevel::new("app", r(15, 18)),
        ];
        let err = validate_chain("password.length", &chain).unwrap_err();
        match err {
            CoreError::PolicyViolation { field, .. } => {
                assert!(field.contains("realm"), "blame should point at realm: {field}");
            }
            other => panic!("expected PolicyViolation, got: {other:?}"),
        }
    }

    #[test]
    fn validate_chain_single_level_is_noop() {
        let chain = vec![PolicyLevel::new("master", r(4, 64))];
        validate_chain("password.length", &chain).unwrap();
    }

    // ---- cascade clamp on master tighten ----

    #[test]
    fn cascade_clamp_rewrites_realm_and_app_to_fit_new_master() {
        let chain = vec![
            PolicyLevel::new("master", r(8, 16)), // newly tightened
            PolicyLevel::new("realm", r(4, 32)),
            PolicyLevel::new("app", r(2, 100)),
        ];
        let (out, changes) = cascade_clamp("password.length", chain);
        // Master is left untouched.
        assert_eq!(out[0].spec, r(8, 16));
        // Realm clamps to master (8..16).
        assert_eq!(out[1].spec, r(8, 16));
        // App must fit the newly-clamped realm, also (8..16).
        assert_eq!(out[2].spec, r(8, 16));
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].level, "realm");
        assert_eq!(changes[0].before, r(4, 32));
        assert_eq!(changes[0].after, r(8, 16));
        assert_eq!(changes[1].level, "app");
        assert_eq!(changes[1].after, r(8, 16));
    }

    #[test]
    fn cascade_clamp_leaves_compliant_levels_alone() {
        let chain = vec![
            PolicyLevel::new("master", r(4, 64)),
            PolicyLevel::new("realm", r(8, 32)), // already inside master
            PolicyLevel::new("app", r(10, 16)),  // already inside realm
        ];
        let (out, changes) = cascade_clamp("password.length", chain.clone());
        assert_eq!(out, chain);
        assert!(changes.is_empty());
    }

    #[test]
    fn cascade_clamp_chain_re_validates_after_cascade() {
        let chain = vec![
            PolicyLevel::new("master", r(8, 16)),
            PolicyLevel::new("realm", r(4, 32)),
            PolicyLevel::new("app", r(2, 100)),
        ];
        let (out, _) = cascade_clamp("password.length", chain);
        validate_chain("password.length", &out).expect("post-cascade chain must validate");
    }

    #[test]
    fn cascade_clamp_handles_toggle_lock_flip() {
        // Master flips from Open → Locked(true). Children's stored
        // Open(default:false) and Locked(value:false) must both clamp
        // to Locked(true).
        let chain = vec![
            PolicyLevel::new(
                "master",
                PolicySpec::Toggle(TogglePolicy::Locked { value: true }),
            ),
            PolicyLevel::new(
                "realm",
                PolicySpec::Toggle(TogglePolicy::Open { default: false }),
            ),
            PolicyLevel::new(
                "app",
                PolicySpec::Toggle(TogglePolicy::Locked { value: false }),
            ),
        ];
        let (out, changes) = cascade_clamp("require_email_verified", chain);
        let locked_true = PolicySpec::Toggle(TogglePolicy::Locked { value: true });
        assert_eq!(out[1].spec, locked_true);
        assert_eq!(out[2].spec, locked_true);
        assert_eq!(changes.len(), 2);
    }

    #[test]
    fn cascade_clamp_handles_enum_removal() {
        // Master drops "facebook" from the allowed providers. Realm and
        // app must lose it on their next read, and the audit log gets
        // an entry per affected level.
        let chain = vec![
            PolicyLevel::new(
                "master",
                PolicySpec::EnumSet(EnumSetPolicy::new(["google", "github"])),
            ),
            PolicyLevel::new(
                "realm",
                PolicySpec::EnumSet(EnumSetPolicy::new(["google", "github", "facebook"])),
            ),
            PolicyLevel::new(
                "app",
                PolicySpec::EnumSet(EnumSetPolicy::new(["facebook"])),
            ),
        ];
        let (out, changes) = cascade_clamp("oauth.providers", chain);
        let realm = match &out[1].spec {
            PolicySpec::EnumSet(s) => s,
            _ => panic!("realm should remain EnumSet"),
        };
        assert!(!realm.allowed.contains("facebook"));
        assert!(realm.allowed.contains("google"));
        let app = match &out[2].spec {
            PolicySpec::EnumSet(s) => s,
            _ => panic!("app should remain EnumSet"),
        };
        assert!(app.allowed.is_empty(), "facebook was its only entry");
        assert_eq!(changes.len(), 2);
        for c in &changes {
            assert_eq!(c.field, "oauth.providers");
        }
    }
}
