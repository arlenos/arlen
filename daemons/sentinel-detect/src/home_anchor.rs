//! Learn where the machine sleeps, so a tag seen there can be damped
//! (`tracker-sentinel-plan.md` §2.4).
//!
//! A laptop has a dominant resting place. Your own tag - keys, backpack - is
//! co-present there; a stalking tag is present at your away-locations and NOT at
//! your home base. That asymmetry buys two things at once: a tag seen at home is
//! very likely a household member's or your own, so it is damped to review, and the
//! neighbour's tag through the wall is by definition only ever at the home place,
//! so it never reaches two distinct locations and never alarms.
//!
//! **Learned, never asked.** No address entry, no place picker, nothing a person
//! types and nothing that leaves the machine. The anchor is the coarse place the
//! machine is at most often at night, and "at night" is what makes it the sleeping
//! place rather than the desk somebody works from.
//!
//! **Coarse, and only ever compared.** The anchor is a fix like any other and is
//! used through [`crate::movement::is_distinct_place`] - the same 400 m question the
//! criteria ask - so nothing here holds a precision the rest of the system does not.
//!
//! **It takes evidence before it answers.** An anchor learned from one night is a
//! hotel room, and treating a hotel room as home would damp exactly the tag somebody
//! most needs to hear about. Below [`MIN_NIGHTS`] distinct nights,
//! [`HomeAnchor::place`] answers `None` and every tag is judged as if away from
//! home, which is the fail-toward-telling-you direction.

use std::collections::BTreeMap;

use crate::movement::{is_distinct_place, Fix};

/// Nights counted before an anchor is trusted. Three distinct nights at the same
/// place is a home; two is a weekend away.
pub const MIN_NIGHTS: u32 = 3;

/// The hours that count as night, local time, as whole hours since midnight. The
/// window is deliberately narrow: the question is where the machine SLEEPS, and an
/// evening at a friend's would answer a wider one.
pub const NIGHT_FROM_HOUR: u32 = 1;
/// Exclusive end of the night window.
pub const NIGHT_TO_HOUR: u32 = 5;

/// Whether a local hour-of-day falls in the night window.
pub fn is_night_hour(hour: u32) -> bool {
    (NIGHT_FROM_HOUR..NIGHT_TO_HOUR).contains(&hour)
}

/// One night's resting place.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NightFix {
    /// Which night, as a day number - the daemon's own count, so two fixes on one
    /// night cannot be counted as two nights.
    pub night: u64,
    /// Where the machine was.
    pub fix: Fix,
}

/// The learned resting place.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct HomeAnchor {
    /// One fix per night, newest last. Bounded by [`Self::MAX_NIGHTS`].
    nights: Vec<(u64, [f64; 2])>,
}

impl HomeAnchor {
    /// How many nights are kept. Enough that a fortnight away does not erase a
    /// home, few enough that a move to a new flat becomes the answer within weeks.
    pub const MAX_NIGHTS: usize = 30;

    /// Record where the machine was on one night. A second fix for a night already
    /// recorded replaces it rather than counting twice - the night is the unit.
    pub fn record(&mut self, night: NightFix) {
        if !night.fix.lat.is_finite() || !night.fix.lon.is_finite() {
            return;
        }
        let entry = (night.night, [night.fix.lat, night.fix.lon]);
        match self.nights.iter_mut().find(|(n, _)| *n == night.night) {
            Some(slot) => *slot = entry,
            None => {
                if self.nights.len() == Self::MAX_NIGHTS {
                    self.nights.remove(0);
                }
                self.nights.push(entry);
            }
        }
    }

    /// How many nights have been recorded.
    pub fn nights(&self) -> usize {
        self.nights.len()
    }

    /// The learned home, or `None` while the evidence is thin.
    ///
    /// The answer is the place the most nights cluster at, where a place is "within
    /// 400 m of this one" - the same question the criteria ask, so the anchor cannot
    /// be more precise than the thing it is compared against. Ties go to the place
    /// whose first night is earliest, so the same history always gives the same
    /// answer.
    pub fn place(&self) -> Option<Fix> {
        if (self.nights.len() as u32) < MIN_NIGHTS {
            return None;
        }
        let fixes: Vec<Fix> = self.nights.iter().map(|(_, f)| Fix { lat: f[0], lon: f[1] }).collect();
        let mut counts: BTreeMap<usize, u32> = BTreeMap::new();
        for (i, anchor) in fixes.iter().enumerate() {
            let seen = fixes.iter().filter(|f| !is_distinct_place(*anchor, **f)).count() as u32;
            counts.insert(i, seen);
        }
        let best = counts.iter().max_by_key(|(i, c)| (**c, std::cmp::Reverse(**i)))?;
        let count = *best.1;
        if count < MIN_NIGHTS {
            // Enough nights recorded, but spread over places - somebody travelling.
            return None;
        }
        Some(fixes[*best.0])
    }

    /// Whether `fix` is at the learned home. `false` while the anchor is unlearned,
    /// which judges every tag as if away from home: the direction that tells you
    /// about a tag rather than damping it.
    pub fn is_home(&self, fix: Fix) -> bool {
        match self.place() {
            Some(home) => !is_distinct_place(home, fix),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: Fix = Fix { lat: 47.2692, lon: 11.4041 };
    const AWAY: Fix = Fix { lat: 47.2692, lon: 11.4570 };

    fn nights(anchor: &mut HomeAnchor, from: u64, count: u64, fix: Fix) {
        for n in from..from + count {
            anchor.record(NightFix { night: n, fix });
        }
    }

    #[test]
    fn the_night_window_is_the_sleeping_hours() {
        assert!(is_night_hour(1) && is_night_hour(4));
        assert!(!is_night_hour(0), "midnight is still an evening");
        assert!(!is_night_hour(5) && !is_night_hour(22));
    }

    #[test]
    fn one_night_is_a_hotel_room_and_answers_nothing() {
        let mut a = HomeAnchor::default();
        nights(&mut a, 1, 1, HOME);
        assert_eq!(a.place(), None);
        assert!(!a.is_home(HOME), "an unlearned anchor damps nothing");
    }

    #[test]
    fn three_nights_in_one_place_is_a_home() {
        let mut a = HomeAnchor::default();
        nights(&mut a, 1, 3, HOME);
        assert_eq!(a.place(), Some(HOME));
        assert!(a.is_home(HOME));
        assert!(!a.is_home(AWAY));
    }

    /// Somebody travelling has nights, and no home among them.
    #[test]
    fn nights_spread_across_places_answer_nothing() {
        let mut a = HomeAnchor::default();
        a.record(NightFix { night: 1, fix: HOME });
        a.record(NightFix { night: 2, fix: AWAY });
        a.record(NightFix { night: 3, fix: Fix { lat: 48.2082, lon: 16.3738 } });
        assert_eq!(a.place(), None);
    }

    /// A fortnight away does not erase a home learned over months.
    #[test]
    fn the_dominant_place_wins_over_a_stretch_away() {
        let mut a = HomeAnchor::default();
        nights(&mut a, 1, 10, HOME);
        nights(&mut a, 11, 4, AWAY);
        assert_eq!(a.place(), Some(HOME));
    }

    #[test]
    fn a_night_recorded_twice_counts_once() {
        let mut a = HomeAnchor::default();
        a.record(NightFix { night: 1, fix: HOME });
        a.record(NightFix { night: 1, fix: AWAY });
        assert_eq!(a.nights(), 1);
        assert_eq!(a.place(), None, "one night is still one night");
    }

    #[test]
    fn the_history_is_bounded_and_a_move_becomes_the_answer() {
        let mut a = HomeAnchor::default();
        nights(&mut a, 1, HomeAnchor::MAX_NIGHTS as u64, HOME);
        assert_eq!(a.nights(), HomeAnchor::MAX_NIGHTS);
        nights(&mut a, 100, HomeAnchor::MAX_NIGHTS as u64, AWAY);
        assert_eq!(a.nights(), HomeAnchor::MAX_NIGHTS);
        assert_eq!(a.place(), Some(AWAY), "the old flat aged out");
    }

    #[test]
    fn a_non_finite_fix_is_not_recorded() {
        let mut a = HomeAnchor::default();
        a.record(NightFix { night: 1, fix: Fix { lat: f64::NAN, lon: 11.4 } });
        assert_eq!(a.nights(), 0);
    }
}
