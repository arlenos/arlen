//! Graph-drift merge primitives (graph-drift.md §2).
//!
//! The four bi-temporal wall-clock stamps (`valid_at`/`invalid_at`/`created_at`/
//! `expired_at`) are human-meaningful times, but they cannot safely ORDER writes
//! that originate on different devices: two machines' clocks disagree, and two
//! writes at the same wall-clock instant have no tiebreak. The hybrid logical
//! clock (HLC) here is the net-new merge order: it fuses physical micros with a
//! logical counter so concurrent same-instant writes still totally order, and it
//! is monotonic across a receive so a device that sees a future remote stamp
//! never issues an earlier one afterwards.
//!
//! Pure + net-new. Its consumers (a per-edge HLC column, `resolve_membership`'s
//! trust-then-HLC winner rule, the write-socket origin) land in later GD slices,
//! so this is `#[allow(dead_code)]` until then, built mechanism-before-trigger
//! like the executor/compensation/canary cores.
#![allow(dead_code)]

/// A hybrid logical clock timestamp (Kulkarni et al.): physical wall-clock
/// micros fused with a logical counter.
///
/// The ordering is `(physical, logical)` lexicographically, which the derived
/// `Ord` gives for free since the fields are declared in that order. Two HLCs
/// that compare equal are genuinely concurrent same-instant writes; the merge
/// layer breaks that tie with the asserting device id (a separate stable
/// column), never with the wall clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hlc {
    /// Physical wall-clock component, micros since the epoch (the same unit the
    /// bi-temporal stamps use). Carried so an HLC stays human-readable and a
    /// live clock still drives ordering when writes are far apart in real time.
    pub physical: u64,
    /// Logical counter, advanced when two events share a physical micro (or when
    /// a local clock has not moved since the last event) so ordering stays
    /// total. Reset to 0 whenever the physical component strictly advances.
    pub logical: u32,
}

impl Hlc {
    /// The zero clock: before any event. `tick`/`merge` from here adopt the
    /// supplied physical time.
    pub const ZERO: Hlc = Hlc {
        physical: 0,
        logical: 0,
    };

    /// Construct an HLC from its two components (e.g. reading a stored column).
    pub fn new(physical: u64, logical: u32) -> Hlc {
        Hlc { physical, logical }
    }

    /// Advance the clock for a LOCAL event at physical time `now` (micros).
    ///
    /// If wall time has moved past the last physical component the counter
    /// resets; otherwise (same micro, or a clock that has not advanced / has
    /// gone backwards) the counter increments so the new stamp still strictly
    /// follows the last. The result is always `>= self`, so a device never
    /// issues a non-increasing local stamp even under a backwards clock.
    pub fn tick(self, now: u64) -> Hlc {
        let physical = self.physical.max(now);
        let logical = if physical == self.physical {
            self.logical.saturating_add(1)
        } else {
            0
        };
        Hlc { physical, logical }
    }

    /// Advance the clock on RECEIVING a `remote` HLC, at local physical time
    /// `now` (micros). The standard HLC receive rule: take the max physical of
    /// all three, then pick the logical counter from whichever input(s) own that
    /// physical component, +1. The result is monotonic: strictly greater than
    /// both `self` and `remote`, so ordering never regresses after a merge.
    pub fn merge(self, remote: Hlc, now: u64) -> Hlc {
        let physical = self.physical.max(remote.physical).max(now);
        let logical = if physical == self.physical && physical == remote.physical {
            self.logical.max(remote.logical).saturating_add(1)
        } else if physical == self.physical {
            self.logical.saturating_add(1)
        } else if physical == remote.physical {
            remote.logical.saturating_add(1)
        } else {
            0
        };
        Hlc { physical, logical }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordering_is_physical_then_logical() {
        // Physical dominates the logical counter.
        assert!(Hlc::new(10, 0) < Hlc::new(11, 0));
        assert!(Hlc::new(10, 5) < Hlc::new(11, 0));
        // Same physical: the logical counter decides.
        assert!(Hlc::new(10, 0) < Hlc::new(10, 1));
        assert_eq!(Hlc::new(10, 3), Hlc::new(10, 3));
    }

    #[test]
    fn tick_resets_logical_when_wall_time_advances() {
        let a = Hlc::new(100, 7);
        let b = a.tick(200);
        assert_eq!(b, Hlc::new(200, 0));
        assert!(b > a);
    }

    #[test]
    fn tick_increments_logical_at_the_same_micro() {
        let a = Hlc::new(100, 0);
        let b = a.tick(100);
        assert_eq!(b, Hlc::new(100, 1));
        assert!(b > a);
    }

    #[test]
    fn tick_is_monotonic_under_a_backwards_clock() {
        // The clock jumps back (100 -> 50): the stamp must still advance.
        let a = Hlc::new(100, 0);
        let b = a.tick(50);
        assert_eq!(b, Hlc::new(100, 1));
        assert!(b > a);
    }

    #[test]
    fn merge_adopts_a_future_remote_and_stays_ahead_of_both() {
        let local = Hlc::new(100, 4);
        let remote = Hlc::new(300, 2);
        let merged = local.merge(remote, 120);
        // Remote physical wins; its logical is the base + 1.
        assert_eq!(merged, Hlc::new(300, 3));
        assert!(merged > local);
        assert!(merged > remote);
    }

    #[test]
    fn merge_at_a_shared_physical_takes_the_higher_logical_plus_one() {
        let local = Hlc::new(200, 5);
        let remote = Hlc::new(200, 9);
        let merged = local.merge(remote, 200);
        assert_eq!(merged, Hlc::new(200, 10));
        assert!(merged > local);
        assert!(merged > remote);
    }

    #[test]
    fn merge_with_a_stale_remote_advances_local() {
        let local = Hlc::new(500, 2);
        let remote = Hlc::new(100, 0);
        // now == local physical: local owns it, logical + 1.
        let merged = local.merge(remote, 500);
        assert_eq!(merged, Hlc::new(500, 3));
        assert!(merged > local);
        assert!(merged > remote);
    }

    #[test]
    fn merge_when_local_wall_clock_leads_both_resets_logical() {
        let local = Hlc::new(100, 6);
        let remote = Hlc::new(100, 6);
        // A fresh wall time past both: pure physical advance, counter resets.
        let merged = local.merge(remote, 400);
        assert_eq!(merged, Hlc::new(400, 0));
        assert!(merged > local);
    }
}
