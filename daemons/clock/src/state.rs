//! The state the daemon serves, in one read.
//!
//! Every field here is an **anchor**, never a counter: when an alarm next
//! rings, when a timer ends, when a run started. A counter sent over IPC is a
//! number that was true when it was sent and is wrong by the time it is drawn;
//! an anchor stays true, and the view derives the countdown from it against the
//! wall clock. The app is already built that way, so handing back a remaining
//! count would not just be untidy - it would be a different contract.
//!
//! The one place a count appears is a **paused** timer, and that is the same
//! rule rather than an exception: a paused timer has no end instant, so what is
//! preserved is the snapshot the daemon took when it stopped.

use serde::{Deserialize, Serialize};

pub use crate::alarm::Alarm;

/// One countdown timer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timer {
    /// Stable id.
    pub id: String,
    /// What it was set for.
    pub duration_ms: i64,
    /// Epoch ms it ends, while running. `None` when paused.
    pub ends_at: Option<i64>,
    /// What was left when it was paused. `None` while running - the remaining
    /// time is `ends_at` minus now, and storing both would let them disagree.
    pub remaining_ms: Option<i64>,
    /// Whether it is paused.
    pub paused: bool,
}

/// The focus session. One at a time, by design: two focus sessions is not a
/// state anyone wants to reason about, and the surface offers no way to ask.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FocusSession {
    /// Which half of the cycle this is.
    pub phase: FocusPhase,
    /// Which round, 1-based.
    pub round: u32,
    /// How many rounds the session runs.
    pub rounds: u32,
    /// Epoch ms this phase ends.
    pub ends_at: i64,
    /// **What the enforcement actually suppressed.** Named rather than implied:
    /// the design's rule is that whatever a focus session silences must be
    /// stated honestly and be fully reversible, so this lists what is held and
    /// the surface reports it rather than asserting the session "blocks
    /// distractions".
    pub held: Vec<String>,
}

/// Which half of a focus cycle is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusPhase {
    /// Working.
    Focus,
    /// Resting.
    Break,
}

/// How the focus cycle is configured, in minutes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FocusConfig {
    /// Length of a focus phase.
    pub focus_min: u32,
    /// Length of a break phase.
    pub break_min: u32,
    /// How many rounds a session runs.
    pub rounds: u32,
}

impl Default for FocusConfig {
    /// The Pomodoro defaults, which is what the app renders before anyone
    /// changes anything.
    fn default() -> Self {
        Self {
            focus_min: 25,
            break_min: 5,
            rounds: 4,
        }
    }
}

/// The stopwatch.
///
/// `started_at` plus `accumulated_ms` rather than an elapsed count, and the
/// pause is **daemon-side on purpose**: `CLOCK_MONOTONIC` is only guaranteed to
/// stop during suspend-to-RAM, and s2idle - what most 2024-and-later laptops
/// actually enter - keeps it running. A stopwatch that trusted the kernel there
/// would quietly count the hours a closed laptop spent asleep.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Stopwatch {
    /// Whether it is running.
    pub running: bool,
    /// Epoch ms the current run began. `None` when paused or reset.
    pub started_at: Option<i64>,
    /// Milliseconds from completed runs.
    pub accumulated_ms: i64,
    /// Lap totals in ms, oldest first.
    pub laps: Vec<i64>,
}

/// One world-clock city.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldCity {
    /// Stable id.
    pub id: String,
    /// What to call it.
    pub name: String,
    /// IANA zone, e.g. `Asia/Tokyo`.
    pub zone: String,
}

/// Everything the app reads in one call.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ClockState {
    /// **Whether alarms can wake this machine from sleep.**
    ///
    /// A state, not an error. `CAP_WAKE_ALARM` is probed rather than assumed -
    /// it is absent over SSH, absent on older systemd, and absent if a rebuild
    /// dropped the file capability - and when it is absent the honest answer is
    /// a permanent, visible "this machine will not be woken", surfaced when the
    /// alarm is set rather than discovered when it fails to ring. A clock that
    /// silently cannot wake you is worse than one that says so.
    pub wake_capable: bool,
    /// The alarms, in the order the user arranged them.
    pub alarms: Vec<Alarm>,
    /// The countdown timers.
    pub timers: Vec<Timer>,
    /// The focus session, if one is running.
    pub focus: Option<FocusSession>,
    /// How focus sessions are configured.
    pub focus_config: FocusConfig,
    /// The stopwatch.
    pub stopwatch: Stopwatch,
    /// The world clocks on show.
    pub world: Vec<WorldCity>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn keys(value: &Value) -> Vec<String> {
        let mut k: Vec<String> = value
            .as_object()
            .expect("an object")
            .keys()
            .cloned()
            .collect();
        k.sort();
        k
    }

    fn sorted(names: &[&str]) -> Vec<String> {
        let mut v: Vec<String> = names.iter().map(|s| s.to_string()).collect();
        v.sort();
        v
    }

    /// **The field names are the contract**, and the app is already written
    /// against them. A rename here does not fail a build - it arrives in the
    /// view as `undefined`, which renders as a blank rather than as an error.
    /// That is the class of bug that reads as "not implemented yet", so it is
    /// pinned rather than trusted.
    #[test]
    fn the_served_shape_is_the_one_the_app_reads() {
        let state = ClockState::default();
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(
            keys(&json),
            sorted(&[
                "wake_capable",
                "alarms",
                "timers",
                "focus",
                "focus_config",
                "stopwatch",
                "world"
            ])
        );
        assert_eq!(
            keys(&serde_json::to_value(FocusConfig::default()).unwrap()),
            sorted(&["focus_min", "break_min", "rounds"])
        );
        assert_eq!(
            keys(&serde_json::to_value(Stopwatch::default()).unwrap()),
            sorted(&["running", "started_at", "accumulated_ms", "laps"])
        );
    }

    #[test]
    fn a_timer_carries_its_anchor_and_its_paused_snapshot_by_name() {
        let t = Timer {
            id: "t1".into(),
            duration_ms: 1_500_000,
            ends_at: Some(42),
            remaining_ms: None,
            paused: false,
        };
        let json = serde_json::to_value(&t).unwrap();
        assert_eq!(
            keys(&json),
            sorted(&["id", "duration_ms", "ends_at", "remaining_ms", "paused"])
        );
        // Absent means absent, not zero: a running timer with `remaining_ms: 0`
        // would render as finished.
        assert!(json["remaining_ms"].is_null());
    }

    #[test]
    fn a_focus_session_names_what_it_holds() {
        let f = FocusSession {
            phase: FocusPhase::Break,
            round: 2,
            rounds: 4,
            ends_at: 99,
            held: vec!["notifications".into()],
        };
        let json = serde_json::to_value(&f).unwrap();
        assert_eq!(
            keys(&json),
            sorted(&["phase", "round", "rounds", "ends_at", "held"])
        );
        // The app matches on these two strings.
        assert_eq!(json["phase"], "break");
        assert_eq!(
            serde_json::to_value(FocusPhase::Focus).unwrap(),
            Value::from("focus")
        );
    }

    #[test]
    fn the_whole_state_survives_the_wire() {
        let state = ClockState {
            wake_capable: false,
            alarms: vec![],
            timers: vec![Timer {
                id: "t".into(),
                duration_ms: 1,
                ends_at: None,
                remaining_ms: Some(1),
                paused: true,
            }],
            focus: None,
            focus_config: FocusConfig::default(),
            stopwatch: Stopwatch::default(),
            world: vec![WorldCity {
                id: "w".into(),
                name: "Tokyo".into(),
                zone: "Asia/Tokyo".into(),
            }],
        };
        let back: ClockState =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        assert_eq!(back, state);
    }

    /// A machine that cannot be woken is the default until the probe says
    /// otherwise, so a daemon that failed to probe reports the honest state
    /// rather than the flattering one.
    #[test]
    fn not_being_able_to_wake_is_the_default() {
        assert!(!ClockState::default().wake_capable);
    }
}
