//! Per-VM sandbox limits for the JS runtime.
//!
//! Three kinds of bound apply to one `AppHooks`:
//!
//! - **Memory** (`memory_bytes`) — QuickJS hard cap. Allocations past the
//!   ceiling fail; the offending hook surfaces as `RuntimeError::Js`.
//! - **Stack** (`stack_bytes`) — JS recursion depth.
//! - **CPU time** (`cpu_time_ms`) — wall-clock budget for *one* JS entry
//!   from Rust (one `eval`, one `dispatch*`). Enforced via QuickJS's
//!   interrupt handler: a shared `AtomicU64` deadline is armed before
//!   each entry and disarmed after. The handler is invoked by QuickJS
//!   periodically (every few thousand bytecode ops) and returns `true`
//!   once the deadline passes, which aborts the running JS.
//!
//! Network and FS access are not policed here — they're simply not
//! exposed in the `$app` global. Adding them later (e.g. a gated
//! `$app.fetch`) is where the allowlist plumbing would live.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Default ceilings for a freshly-created `AppHooks` when the caller
/// supplies no policy. These are conservative enough to keep one
/// runaway hook from taking down the server, generous enough that a
/// normal record CRUD hook never notices them.
#[derive(Debug, Clone, Copy)]
pub struct SandboxLimits {
    /// Max heap (bytes) for the QuickJS runtime. `None` = unlimited.
    pub memory_bytes: Option<usize>,
    /// Max stack (bytes) for the QuickJS runtime. `None` = QuickJS
    /// default (256 KiB).
    pub stack_bytes: Option<usize>,
    /// Wall-clock budget (ms) for one JS entry. `None` = no deadline.
    pub cpu_time_ms: Option<u64>,
}

impl Default for SandboxLimits {
    fn default() -> Self {
        Self {
            memory_bytes: Some(64 * 1024 * 1024), // 64 MiB
            stack_bytes: Some(1 * 1024 * 1024),   //  1 MiB
            cpu_time_ms: Some(1_000),             //  1 s
        }
    }
}

impl SandboxLimits {
    /// Disable all ceilings. Useful for trusted bootstrap code and for
    /// tests that need to allocate or loop without hitting policy.
    pub fn unlimited() -> Self {
        Self {
            memory_bytes: None,
            stack_bytes: None,
            cpu_time_ms: None,
        }
    }
}

/// Shared state between the interrupt handler and the arm/disarm logic.
///
/// `start` is captured once when the AppHooks is built; `deadline_ms`
/// stores "milliseconds since `start`" at which point a JS execution
/// should be interrupted. `0` means no deadline armed.
#[derive(Clone)]
pub(crate) struct CpuClock {
    pub start: Instant,
    pub deadline_ms: Arc<AtomicU64>,
}

impl CpuClock {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            deadline_ms: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Arm a deadline `budget_ms` from now. Returns a guard that
    /// disarms on drop so the next JS call doesn't inherit a stale
    /// deadline (or worse, an already-expired one).
    pub fn arm(&self, budget_ms: u64) -> CpuGuard<'_> {
        let now = self.start.elapsed().as_millis() as u64;
        // Saturating add: catastrophic but predictable on overflow
        // (just means "no early interrupt").
        let target = now.saturating_add(budget_ms);
        self.deadline_ms.store(target, Ordering::Relaxed);
        CpuGuard { clock: self }
    }

    /// True if a deadline is currently armed AND has been crossed.
    /// Used by the entry points to convert a generic JS error into a
    /// clearer `RuntimeError::Timeout` when the cause was almost
    /// certainly the interrupt firing.
    pub fn deadline_crossed(&self) -> bool {
        let d = self.deadline_ms.load(Ordering::Relaxed);
        if d == 0 {
            return false;
        }
        self.start.elapsed().as_millis() as u64 >= d
    }
}

/// RAII handle that disarms the CPU deadline on drop. Holding one
/// across a JS entry guarantees the next entry starts clean.
pub(crate) struct CpuGuard<'a> {
    clock: &'a CpuClock,
}

impl Drop for CpuGuard<'_> {
    fn drop(&mut self) {
        self.clock.deadline_ms.store(0, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn default_limits_are_nonzero() {
        let d = SandboxLimits::default();
        assert!(d.memory_bytes.is_some());
        assert!(d.stack_bytes.is_some());
        assert!(d.cpu_time_ms.is_some());
    }

    #[test]
    fn unlimited_clears_all() {
        let u = SandboxLimits::unlimited();
        assert!(u.memory_bytes.is_none());
        assert!(u.stack_bytes.is_none());
        assert!(u.cpu_time_ms.is_none());
    }

    #[test]
    fn cpu_clock_unarmed_does_not_cross() {
        let c = CpuClock::new();
        assert!(!c.deadline_crossed());
    }

    #[test]
    fn cpu_clock_arm_then_sleep_then_check_crosses() {
        let c = CpuClock::new();
        let _g = c.arm(5);
        thread::sleep(Duration::from_millis(20));
        assert!(c.deadline_crossed());
    }

    #[test]
    fn cpu_clock_disarms_on_guard_drop() {
        let c = CpuClock::new();
        {
            let _g = c.arm(5);
            thread::sleep(Duration::from_millis(20));
            assert!(c.deadline_crossed());
        }
        // Guard dropped → deadline cleared → no more "crossed".
        assert!(!c.deadline_crossed());
    }
}
