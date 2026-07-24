//! PWR-R3 idle policy: the pure idle-escalation core.
//!
//! The daemon watches the compositor's `ext-idle-notify-v1` and applies an
//! escalating set of actions the longer the seat stays idle (dim, then
//! blank the screen, optionally lock, optionally suspend). This module is
//! the pure DECISION half: it turns the configured per-stage thresholds
//! into an ordered set of [`IdleStage`]s the daemon registers one idle
//! timer per. The idle-notify Wayland client and the action executors
//! (brightness, DPMS/blank, lock, suspend) are the wiring half that
//! consumes these stages, kept separate so the escalation logic is
//! unit-testable without a compositor.
//!
//! Dim and Blank are NON-destructive (any input restores the screen), so
//! they default ON - the standard desktop idle behaviour. Lock and Suspend
//! are higher-impact, so they default OFF and are opt-in, matching the
//! daemon's conservative posture for actions a user would not want to hit
//! unexpectedly (`config::CriticalActionConfig`).

/// One idle-escalation action, applied when its stage's idle timer fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleAction {
    /// Reduce the screen brightness to `to_percent` (restored on resume).
    Dim {
        /// The brightness percent to dim to (0..=100).
        to_percent: u8,
    },
    /// Turn the screen off via the compositor / DPMS (restored on resume).
    Blank,
    /// Engage the lock screen.
    Lock,
    /// Suspend the machine (logind Suspend).
    Suspend,
}

/// One escalation stage: after `after_secs` of continuous idle, apply
/// `action`. The daemon registers one `ext-idle-notify-v1` timer per stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdleStage {
    /// Seconds of continuous idle before this stage fires.
    pub after_secs: u32,
    /// What to do when it fires.
    pub action: IdleAction,
}

/// Build the ordered idle-escalation stages from the configured thresholds.
/// A threshold of `0` disables that stage; `dim_to` is clamped to `0..=100`.
/// The result is sorted ascending by threshold - the order the daemon
/// registers and fires them - with a stable sort so that on a shared
/// threshold the natural escalation order (dim, blank, lock, suspend) is
/// preserved.
pub fn stages(
    dim_after: u32,
    dim_to: u8,
    blank_after: u32,
    lock_after: u32,
    suspend_after: u32,
) -> Vec<IdleStage> {
    let mut stages = Vec::new();
    if dim_after > 0 {
        stages.push(IdleStage {
            after_secs: dim_after,
            action: IdleAction::Dim {
                to_percent: dim_to.min(100),
            },
        });
    }
    if blank_after > 0 {
        stages.push(IdleStage {
            after_secs: blank_after,
            action: IdleAction::Blank,
        });
    }
    if lock_after > 0 {
        stages.push(IdleStage {
            after_secs: lock_after,
            action: IdleAction::Lock,
        });
    }
    if suspend_after > 0 {
        stages.push(IdleStage {
            after_secs: suspend_after,
            action: IdleAction::Suspend,
        });
    }
    // Stable sort: equal thresholds keep the dim<blank<lock<suspend order
    // they were pushed in, so a shared threshold escalates sensibly.
    stages.sort_by_key(|s| s.after_secs);
    stages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_thresholds_yield_dim_then_blank() {
        // The shipped defaults (dim 300, blank 600, lock/suspend off).
        let s = stages(300, 30, 600, 0, 0);
        assert_eq!(
            s,
            vec![
                IdleStage {
                    after_secs: 300,
                    action: IdleAction::Dim { to_percent: 30 }
                },
                IdleStage {
                    after_secs: 600,
                    action: IdleAction::Blank
                },
            ]
        );
    }

    #[test]
    fn all_stages_enabled_sort_ascending() {
        // Configured out of order; the result is sorted by threshold.
        let s = stages(300, 40, 600, 900, 1800);
        let seconds: Vec<u32> = s.iter().map(|st| st.after_secs).collect();
        assert_eq!(seconds, vec![300, 600, 900, 1800]);
        assert_eq!(s.last().unwrap().action, IdleAction::Suspend);
    }

    #[test]
    fn a_zero_threshold_disables_its_stage() {
        // Only blank enabled.
        let s = stages(0, 30, 600, 0, 0);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].action, IdleAction::Blank);

        // Everything off -> no stages (the daemon registers nothing).
        assert!(stages(0, 30, 0, 0, 0).is_empty());
    }

    #[test]
    fn dim_percent_is_clamped() {
        let s = stages(300, 250, 0, 0, 0);
        assert_eq!(s[0].action, IdleAction::Dim { to_percent: 100 });
    }

    #[test]
    fn a_shared_threshold_keeps_escalation_order() {
        // dim and lock both at 300: dim (pushed first) sorts before lock.
        let s = stages(300, 30, 0, 300, 0);
        assert_eq!(s[0].action, IdleAction::Dim { to_percent: 30 });
        assert_eq!(s[1].action, IdleAction::Lock);
    }
}
