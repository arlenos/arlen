//! The focus session: phases, rounds, and when it is over.
//!
//! **A focus timer that only counts down is a stopwatch with a theme.** The
//! locked decision is that this one enforces - at minimum holding notifications
//! for the duration - and that whatever it holds is stated honestly and is fully
//! reversible. The enforcement itself belongs to the daemon, which has to ask
//! the notification daemon and be told what it actually got; what lives here is
//! the shape of the session and the one judgement inside it.
//!
//! **That judgement: a session ends when its last focus phase ends.** Four
//! rounds means four stretches of work, not four stretches and a break nobody
//! comes back from. The alternative leaves the app showing a break that belongs
//! to no round and a session that finishes while you are away from it.

use crate::state::{FocusConfig, FocusPhase, FocusSession};

/// Milliseconds in a minute, which is the unit the configuration is in because
/// it is the unit a person sets.
const MIN_MS: i64 = 60_000;

/// Begin a session at its first focus phase.
///
/// `held` is what the enforcement managed to suppress, which the caller has
/// already asked for and been told - it is passed in rather than assumed here,
/// because a session that lists what it holds without checking is exactly the
/// dishonesty the design rules out.
pub fn start(config: &FocusConfig, held: Vec<String>, now_ms: i64) -> FocusSession {
    FocusSession {
        phase: FocusPhase::Focus,
        round: 1,
        rounds: config.rounds.max(1),
        ends_at: now_ms + i64::from(config.focus_min) * MIN_MS,
        held,
    }
}

/// The session after its current phase ends, or `None` when it is over.
///
/// Focus gives way to a break in the same round; a break gives way to the next
/// round's focus. The last round has no trailing break - see the module note -
/// so a session of four rounds is work, break, work, break, work, break, work,
/// and then done.
///
/// `held` carries over rather than being re-derived: the enforcement was granted
/// for the session, and a break in the middle of one is still inside it.
pub fn advance(session: &FocusSession, config: &FocusConfig, now_ms: i64) -> Option<FocusSession> {
    match session.phase {
        FocusPhase::Focus if session.round >= session.rounds => None,
        FocusPhase::Focus => Some(FocusSession {
            phase: FocusPhase::Break,
            ends_at: now_ms + i64::from(config.break_min) * MIN_MS,
            ..session.clone()
        }),
        FocusPhase::Break => Some(FocusSession {
            phase: FocusPhase::Focus,
            round: session.round + 1,
            ends_at: now_ms + i64::from(config.focus_min) * MIN_MS,
            ..session.clone()
        }),
    }
}

/// Whether the current phase has run out.
pub fn phase_elapsed(session: &FocusSession, now_ms: i64) -> bool {
    now_ms >= session.ends_at
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> FocusConfig {
        FocusConfig {
            focus_min: 25,
            break_min: 5,
            rounds: 4,
        }
    }

    #[test]
    fn a_session_starts_working_rather_than_resting() {
        let s = start(&config(), vec![], 0);
        assert_eq!(s.phase, FocusPhase::Focus);
        assert_eq!(s.round, 1);
        assert_eq!(s.rounds, 4);
        assert_eq!(s.ends_at, 25 * MIN_MS);
    }

    /// What it holds is passed in, because a session that lists suppressions it
    /// never confirmed is the dishonesty the design rules out.
    #[test]
    fn a_session_carries_what_the_enforcement_actually_held() {
        let s = start(&config(), vec!["notifications".into()], 0);
        assert_eq!(s.held, vec!["notifications".to_string()]);
        let nothing = start(&config(), vec![], 0);
        assert!(
            nothing.held.is_empty(),
            "held nothing, so it claims nothing"
        );
    }

    #[test]
    fn focus_gives_way_to_a_break_in_the_same_round() {
        let s = start(&config(), vec![], 0);
        let next = advance(&s, &config(), 25 * MIN_MS).unwrap();
        assert_eq!(next.phase, FocusPhase::Break);
        assert_eq!(next.round, 1);
        assert_eq!(next.ends_at, 25 * MIN_MS + 5 * MIN_MS);
    }

    #[test]
    fn a_break_gives_way_to_the_next_rounds_focus() {
        let mut s = start(&config(), vec![], 0);
        s.phase = FocusPhase::Break;
        let next = advance(&s, &config(), 100).unwrap();
        assert_eq!(next.phase, FocusPhase::Focus);
        assert_eq!(next.round, 2);
        assert_eq!(next.ends_at, 100 + 25 * MIN_MS);
    }

    /// The judgement in the module note: four rounds is four stretches of work,
    /// and the session ends when the last one does.
    #[test]
    fn the_last_round_ends_the_session_rather_than_starting_a_break() {
        let mut s = start(&config(), vec![], 0);
        s.round = 4;
        assert_eq!(advance(&s, &config(), 100), None);
    }

    /// Walking a whole session through: work and break alternating, ending on
    /// work, with the rounds counted the way a person would count them.
    #[test]
    fn a_whole_session_alternates_and_ends_on_work() {
        let cfg = config();
        let mut s = start(&cfg, vec![], 0);
        let mut seen = vec![(s.phase, s.round)];
        let mut now = 0;
        while let Some(next) = advance(&s, &cfg, now) {
            now += 1;
            s = next;
            seen.push((s.phase, s.round));
        }
        use FocusPhase::{Break, Focus};
        assert_eq!(
            seen,
            vec![
                (Focus, 1),
                (Break, 1),
                (Focus, 2),
                (Break, 2),
                (Focus, 3),
                (Break, 3),
                (Focus, 4),
            ]
        );
    }

    /// A configuration nobody could work with must not produce a session that
    /// cannot end: zero rounds becomes one, so the loop above terminates.
    #[test]
    fn a_session_of_no_rounds_is_a_session_of_one() {
        let cfg = FocusConfig {
            rounds: 0,
            ..config()
        };
        let s = start(&cfg, vec![], 0);
        assert_eq!(s.rounds, 1);
        assert_eq!(advance(&s, &cfg, 100), None);
    }

    #[test]
    fn a_phase_is_elapsed_at_its_end_instant() {
        let s = start(&config(), vec![], 0);
        assert!(!phase_elapsed(&s, 25 * MIN_MS - 1));
        assert!(phase_elapsed(&s, 25 * MIN_MS));
    }

    /// Each phase is measured from when it actually starts, so a daemon that
    /// notices a phase ended late does not compound the lateness.
    #[test]
    fn a_phase_is_measured_from_when_it_begins_not_from_the_schedule() {
        let s = start(&config(), vec![], 0);
        // Noticed two minutes late.
        let next = advance(&s, &config(), 27 * MIN_MS).unwrap();
        assert_eq!(next.ends_at, 27 * MIN_MS + 5 * MIN_MS);
    }
}
