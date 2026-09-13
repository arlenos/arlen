//! When one awake-session ends and the next begins
//! (`tracker-sentinel-plan.md` §2.3).
//!
//! SEN-4 redefines "duration" from continuous minutes to a span across epochs,
//! because a laptop suspends and any continuous measure is shredded by the gaps. So
//! the epoch boundary is load-bearing: it is what makes "seen before and after you
//! closed the lid" expressible at all.
//!
//! The plan names four sources in cost order, and this module is the decision over
//! the first three. The fourth, the accelerometer, is deliberately absent: it yields
//! orientation rather than a movement vector, so it is a gate on whether to look,
//! never evidence that the machine moved, and putting it here would give it a vote
//! it has not earned.
//!
//! 1. **Suspend and resume**, the primary boundary. Laptop closed, moved, opened
//!    somewhere else is the natural new-place boundary of a laptop, and it is
//!    cheaper and more reliable than asking a radio.
//! 2. **A location delta past 400 m**, which opens an epoch without a suspend - you
//!    can carry an open laptop across town.
//! 3. **A changed Wi-Fi BSSID set**, the cheap movement hint when no fix is
//!    available: the visible access points change when you move.
//!
//! **A boundary is never closed retroactively.** The epoch id only ever increases,
//! and a sighting is stamped with the epoch current when it was seen. Renumbering
//! afterwards would rewrite what the criteria already counted.
//!
//! **The BSSID set is compared, never kept.** The hint is "is this set different
//! from the last one", so what the tracker holds is the previous set and nothing
//! else - no history of which networks were visible where, which would be a second
//! location log wearing a different hat.

use std::collections::BTreeSet;

use crate::movement::{is_distinct_place, Fix};

/// What happened that might end an epoch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    /// The machine came back from suspend (logind / the power daemon's
    /// `PrepareForSleep` going false).
    Resumed,
    /// A fresh coarse fix arrived.
    Fix {
        /// Latitude in millionths of a degree, so the signal stays comparable.
        lat_micro: i64,
        /// Longitude in millionths of a degree.
        lon_micro: i64,
    },
    /// The visible access points, as BSSID strings.
    Bssids(BTreeSet<String>),
}

impl Signal {
    /// A fix signal from a [`Fix`].
    pub fn fix(fix: Fix) -> Option<Self> {
        if !fix.lat.is_finite() || !fix.lon.is_finite() {
            return None;
        }
        Some(Self::Fix {
            lat_micro: (fix.lat * 1_000_000.0).round() as i64,
            lon_micro: (fix.lon * 1_000_000.0).round() as i64,
        })
    }
}

/// How much of the access-point set has to change to count as movement.
///
/// Not "any change": one neighbour rebooting a router changes the set while you sit
/// still, and an epoch opened for that would let a tag seen twice at one desk look
/// like a tag seen across two sessions. A majority of the set turning over is a
/// different room.
pub const BSSID_CHANGE_RATIO: f64 = 0.5;

/// The current awake-session and what it was last compared against.
#[derive(Debug, Clone, Default)]
pub struct EpochTracker {
    id: u64,
    last_fix: Option<Fix>,
    last_bssids: Option<BTreeSet<String>>,
}

impl EpochTracker {
    /// The epoch a sighting seen now belongs to.
    pub fn current(&self) -> u64 {
        self.id
    }

    /// Feed one signal. Returns `true` when it opened a new epoch.
    pub fn observe(&mut self, signal: Signal) -> bool {
        match signal {
            Signal::Resumed => {
                // Unconditional: the machine was shut, and where it was shut is not
                // evidence about where it opened.
                self.id += 1;
                // The fix and the access points from before the lid closed say
                // nothing about now, so the next one of each is compared against
                // nothing rather than against a stale reading.
                self.last_fix = None;
                self.last_bssids = None;
                true
            }
            Signal::Fix { lat_micro, lon_micro } => {
                let fix = Fix {
                    lat: lat_micro as f64 / 1_000_000.0,
                    lon: lon_micro as f64 / 1_000_000.0,
                };
                let moved = match self.last_fix {
                    Some(previous) => is_distinct_place(previous, fix),
                    None => false,
                };
                self.last_fix = Some(fix);
                if moved {
                    self.id += 1;
                }
                moved
            }
            Signal::Bssids(seen) => {
                let moved = match &self.last_bssids {
                    Some(previous) if !previous.is_empty() && !seen.is_empty() => {
                        let shared = previous.intersection(&seen).count() as f64;
                        let of = previous.len().max(seen.len()) as f64;
                        (1.0 - shared / of) > BSSID_CHANGE_RATIO
                    }
                    // Nothing to compare against, or a scan that saw nothing: an
                    // empty set is "the radio told us nothing", not "no access
                    // points exist", and the two must not read the same.
                    _ => false,
                };
                self.last_bssids = Some(seen);
                if moved {
                    self.id += 1;
                }
                moved
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bssids(names: &[&str]) -> Signal {
        Signal::Bssids(names.iter().map(|s| s.to_string()).collect())
    }

    const HOME: Fix = Fix { lat: 47.2692, lon: 11.4041 };
    const AWAY: Fix = Fix { lat: 47.2692, lon: 11.4570 };

    #[test]
    fn a_resume_always_opens_an_epoch() {
        let mut e = EpochTracker::default();
        assert_eq!(e.current(), 0);
        assert!(e.observe(Signal::Resumed));
        assert_eq!(e.current(), 1);
        assert!(e.observe(Signal::Resumed));
        assert_eq!(e.current(), 2);
    }

    /// The first fix has nothing to compare against and must not open an epoch by
    /// itself.
    #[test]
    fn the_first_fix_is_not_a_move() {
        let mut e = EpochTracker::default();
        assert!(!e.observe(Signal::fix(HOME).unwrap()));
        assert_eq!(e.current(), 0);
    }

    #[test]
    fn a_fix_past_the_threshold_opens_an_epoch_and_a_nearby_one_does_not() {
        let mut e = EpochTracker::default();
        e.observe(Signal::fix(HOME).unwrap());
        assert!(!e.observe(Signal::fix(Fix { lat: 47.2693, lon: 11.4042 }).unwrap()));
        assert!(e.observe(Signal::fix(AWAY).unwrap()));
        assert_eq!(e.current(), 1);
    }

    /// A stale fix from before the lid closed is not compared against the one after.
    #[test]
    fn a_resume_forgets_what_it_was_comparing_against() {
        let mut e = EpochTracker::default();
        e.observe(Signal::fix(HOME).unwrap());
        e.observe(Signal::Resumed);
        assert!(!e.observe(Signal::fix(AWAY).unwrap()), "the first fix after a resume is a baseline");
        assert_eq!(e.current(), 1, "the resume opened one epoch, the fix did not open a second");
    }

    #[test]
    fn a_mostly_unchanged_access_point_set_is_not_movement() {
        let mut e = EpochTracker::default();
        e.observe(bssids(&["a", "b", "c", "d"]));
        // One neighbour's router gone: still the same room.
        assert!(!e.observe(bssids(&["a", "b", "c"])));
        assert_eq!(e.current(), 0);
    }

    #[test]
    fn a_different_room_opens_an_epoch() {
        let mut e = EpochTracker::default();
        e.observe(bssids(&["a", "b", "c", "d"]));
        assert!(e.observe(bssids(&["w", "x", "y", "z"])));
        assert_eq!(e.current(), 1);
    }

    /// A radio that saw nothing is not a new place.
    #[test]
    fn an_empty_scan_is_not_movement() {
        let mut e = EpochTracker::default();
        e.observe(bssids(&["a", "b"]));
        assert!(!e.observe(bssids(&[])));
        assert!(!e.observe(bssids(&["a", "b"])));
        assert_eq!(e.current(), 0);
    }

    #[test]
    fn a_non_finite_fix_is_not_a_signal() {
        assert!(Signal::fix(Fix { lat: f64::NAN, lon: 11.4 }).is_none());
    }

    /// Epoch ids only ever go up, whatever arrives.
    #[test]
    fn the_id_never_goes_backwards() {
        let mut e = EpochTracker::default();
        let mut last = e.current();
        for s in [
            Signal::Resumed,
            Signal::fix(HOME).unwrap(),
            bssids(&["a"]),
            Signal::fix(AWAY).unwrap(),
            bssids(&["x", "y"]),
            Signal::Resumed,
        ] {
            e.observe(s);
            assert!(e.current() >= last);
            last = e.current();
        }
    }
}
