// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! How often to retry a socket that is not answering, and when to say so.
//!
//! Two of the shell's long-lived subscribers - the Event Bus consumer and the
//! notification client - sat in `loop { connect(); warn!(); sleep(2s) }`. With the
//! daemon absent that is a warn line every two seconds forever, plus an info line
//! per attempt, and the failure is invisible inside its own noise. It is the same
//! defect as a silent refusal wearing the opposite costume: the log says something
//! constantly and tells a reader nothing.
//!
//! [`modulesd_client`](crate::modulesd_client) already answers this for a
//! CALL-driven client: say it once, go quiet for a window, refuse in the meantime.
//! A subscriber cannot refuse - it exists to attach the moment the daemon appears -
//! so the same principle takes a different shape here: keep retrying, widen the gap
//! up to a cap, and speak only at the edges. Absent is said once, back is said once.
//!
//! Pure and clock-free: the caller does the sleeping and the connecting, this only
//! decides. That is what makes the widening testable without a test that waits.

use std::time::Duration;

/// First wait after a failure. Matches the 2s both loops used, so a daemon that
/// starts a moment after the shell is still picked up promptly.
const FIRST: Duration = Duration::from_secs(2);

/// The longest we ever wait. A minute is the modulesd window: long enough that an
/// absent daemon costs nothing, short enough that a person who starts one does not
/// wonder whether the shell noticed.
const CAP: Duration = Duration::from_secs(60);

/// What the caller should do about this attempt's outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Next {
    /// How long to wait before trying again.
    pub wait: Duration,
    /// Whether to log this failure. True only for the first of a run.
    pub speak: bool,
}

/// The retry state of one subscriber's connection.
///
/// Deliberately not a builder or a trait: two call sites, one policy, and a knob
/// nobody sets is a knob that drifts from the only value it is ever given.
#[derive(Debug, Default)]
pub struct Backoff {
    /// Consecutive failures. Zero means the last attempt succeeded.
    failures: u32,
}

impl Backoff {
    /// A fresh connection that has not failed yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a failed attempt and say how to proceed.
    ///
    /// Doubles from [`FIRST`] to [`CAP`]: 2, 4, 8, 16, 32, 60, 60 ... The cap is a
    /// clamp rather than a step count so a longer outage cannot walk the interval
    /// out to hours, which is how "quiet" turns into "asleep".
    pub fn failed(&mut self) -> Next {
        let speak = self.failures == 0;
        // The shift is bounded rather than checked: 2s doubled five times is 64s,
        // already past the cap, so nothing above five can change the answer. A
        // subscriber retrying for a month must not shift a `u32` by 40 and come
        // back with a nonsense interval.
        let wait = FIRST.saturating_mul(1u32 << self.failures.min(5)).min(CAP);
        self.failures = self.failures.saturating_add(1);
        Next { wait, speak }
    }

    /// Record a successful connection; returns true if this ends a failing run.
    ///
    /// The caller logs on true, so a daemon that appears later is reported as
    /// arriving. Without it the log has a beginning and no end, and a reader
    /// cannot tell a fixed outage from an ongoing one.
    pub fn succeeded(&mut self) -> bool {
        let was_failing = self.failures > 0;
        self.failures = 0;
        was_failing
    }

    /// Whether the connection is currently in a failing run. For a caller that
    /// wants to downgrade its own per-attempt chatter while a daemon is absent.
    pub fn failing(&self) -> bool {
        self.failures > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_speaks_once_per_outage_and_once_on_return() {
        // The property the whole file exists for: one line for an absent daemon,
        // not one line every two seconds.
        let mut b = Backoff::new();
        assert!(b.failed().speak, "the first failure must be reported");
        for _ in 0..50 {
            assert!(!b.failed().speak, "a continuing outage must stay quiet");
        }
        assert!(b.succeeded(), "the return must be reported");
        assert!(
            !b.succeeded(),
            "a connection that never failed says nothing"
        );
        assert!(b.failed().speak, "and a NEW outage is reported again");
    }

    #[test]
    fn the_wait_widens_to_the_cap_and_stops() {
        let mut b = Backoff::new();
        let waits: Vec<u64> = (0..8).map(|_| b.failed().wait.as_secs()).collect();
        assert_eq!(waits, vec![2, 4, 8, 16, 32, 60, 60, 60]);
    }

    #[test]
    fn a_very_long_outage_still_has_a_sane_interval() {
        // The shift that would overflow. A subscriber left running for weeks must
        // come back to a minute, not to zero or to a century.
        let mut b = Backoff::new();
        for _ in 0..10_000 {
            b.failed();
        }
        assert_eq!(b.failed().wait, CAP);
        assert!(b.failing());
    }

    #[test]
    fn success_resets_the_widening() {
        // An intermittent daemon must not inherit the long interval from an outage
        // that has since ended - otherwise one bad minute makes the next reconnect
        // take a minute for no reason.
        let mut b = Backoff::new();
        for _ in 0..6 {
            b.failed();
        }
        b.succeeded();
        assert_eq!(b.failed().wait, FIRST);
    }
}
