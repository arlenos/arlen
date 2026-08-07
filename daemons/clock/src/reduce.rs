//! Applying a command to the state, and saying what has to happen next.
//!
//! The daemon's whole decision layer: every command the app can send arrives
//! here, the state comes back changed, and the [`Effects`] say what the shell
//! then has to do about it. Keeping that split means the interesting part - what
//! a command does, and whether the wake needs re-arming afterwards - is decided
//! without a bus, a socket or a clock, and the shell is left with the parts that
//! genuinely need them.
//!
//! **The wake is re-derived from the whole state after every command**, never
//! adjusted incrementally. `Power1` holds one wake, so what matters is the
//! earliest moment across everything - and an incremental update is how the
//! evening alarm ends up armed instead of the morning one. Re-deriving is cheap
//! and cannot drift.
//!
//! **Sleep and wake are events, not commands.** They come from the power daemon
//! over the event bus rather than from logind directly, because one component
//! watching `PrepareForSleep` is the same principle as one component arming
//! wakes: a second watcher is a second answer waiting to disagree.

use crate::alarm::next_fire_at;
use crate::focus;
use crate::run;
use crate::state::{Alarm, ClockState, FocusConfig, WorldCity};

/// What the app asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Add or replace an alarm.
    SetAlarm(Alarm),
    /// Arm or disarm one.
    ToggleAlarm { id: String, enabled: bool },
    /// Remove one.
    DeleteAlarm { id: String },
    /// Start a countdown. **The app sends only a duration** - ids are the
    /// daemon's to mint, so two apps starting a timer at the same moment cannot
    /// collide on one, and a view that closes cannot take an id with it.
    TimerStart { id: String, duration_ms: i64 },
    /// Pause or resume one. A single command with the wanted state rather than
    /// two, matching what the app sends: it says where the switch should be,
    /// not which way to move it, so a repeat is a no-op instead of a toggle
    /// that undoes the first press.
    TimerSetPaused { id: String, paused: bool },
    /// Remove one.
    TimerCancel { id: String },
    /// Begin a focus session, with what the enforcement actually held.
    FocusStart { held: Vec<String> },
    /// End the session early.
    FocusEnd,
    /// Change the focus configuration.
    FocusConfigure(FocusConfig),
    /// Start or resume the stopwatch.
    StopwatchStart,
    /// Pause it.
    StopwatchPause,
    /// Record a lap.
    StopwatchLap,
    /// Back to zero.
    StopwatchReset,
    /// Show a city, by its id in the shared offline dataset.
    ///
    /// The app sends an id and the daemon resolves the name and zone, because
    /// the dataset is owned outside the clock (`clock-app.md` §4) and a city
    /// whose name arrived from a caller is a city nothing can check.
    WorldAdd { id: String },
    /// Stop showing one.
    WorldRemove { id: String },
}

/// Things that happened to the machine rather than things the app asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// The machine is going to sleep (`power.suspend`).
    Suspending,
    /// The machine woke (`power.resume`).
    Resumed,
    /// The wall clock jumped, by this many milliseconds.
    ClockStepped { by_ms: i64 },
}

/// What the shell must do after the state changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effects {
    /// The state is different and should be written down. False for a command
    /// that changed nothing, so a repeated no-op does not rewrite the file.
    pub persist: bool,
    /// The moment to ask `Power1` to wake for, or `None` to hold no wake.
    pub wake_at: Option<i64>,
}

/// Apply a command.
///
/// `tz` resolves an alarm's wall-clock time and `cities` looks a world-clock id
/// up in the shared dataset; both are injected so the decisions here are
/// testable without a zone database or a city list.
pub fn apply<Tz: chrono::TimeZone>(
    state: &mut ClockState,
    command: Command,
    tz: &Tz,
    cities: impl Fn(&str) -> Option<WorldCity>,
    now_ms: i64,
) -> Effects {
    let before = state.clone();
    match command {
        Command::SetAlarm(mut alarm) => {
            alarm.next_fire_at = next_fire_at(&alarm, tz, now_ms);
            match state.alarms.iter_mut().find(|a| a.id == alarm.id) {
                Some(existing) => *existing = alarm,
                None => state.alarms.push(alarm),
            }
        }
        Command::ToggleAlarm { id, enabled } => {
            if let Some(a) = state.alarms.iter_mut().find(|a| a.id == id) {
                a.enabled = enabled;
                // Re-derived rather than kept: a disabled alarm has no next
                // ring, and re-enabling one whose moment passed while it was off
                // must not resurrect that moment.
                a.next_fire_at = next_fire_at(a, tz, now_ms);
            }
        }
        Command::DeleteAlarm { id } => state.alarms.retain(|a| a.id != id),
        Command::TimerStart { id, duration_ms } => {
            let timer = run::timer_start(id, duration_ms, now_ms);
            match state.timers.iter_mut().find(|t| t.id == timer.id) {
                Some(existing) => *existing = timer,
                None => state.timers.push(timer),
            }
        }
        Command::TimerSetPaused { id, paused } => {
            if let Some(t) = state.timers.iter_mut().find(|t| t.id == id) {
                if paused {
                    run::timer_pause(t, now_ms);
                } else {
                    run::timer_resume(t, now_ms);
                }
            }
        }
        Command::TimerCancel { id } => state.timers.retain(|t| t.id != id),
        Command::FocusStart { held } => {
            state.focus = Some(focus::start(&state.focus_config, held, now_ms));
        }
        Command::FocusEnd => state.focus = None,
        Command::FocusConfigure(config) => state.focus_config = config,
        Command::StopwatchStart => run::stopwatch_start(&mut state.stopwatch, now_ms),
        Command::StopwatchPause => run::stopwatch_pause(&mut state.stopwatch, now_ms),
        Command::StopwatchLap => run::stopwatch_lap(&mut state.stopwatch, now_ms),
        Command::StopwatchReset => run::stopwatch_reset(&mut state.stopwatch),
        Command::WorldAdd { id } => {
            if !state.world.iter().any(|c| c.id == id) {
                if let Some(city) = cities(&id) {
                    state.world.push(city);
                }
            }
        }
        Command::WorldRemove { id } => state.world.retain(|c| c.id != id),
    }
    effects(&before, state, now_ms)
}

/// Apply something that happened to the machine.
pub fn observe(state: &mut ClockState, event: Event, now_ms: i64) -> Effects {
    let before = state.clone();
    match event {
        Event::Suspending => run::stopwatch_suspended(&mut state.stopwatch, now_ms),
        Event::Resumed => run::stopwatch_resumed(&mut state.stopwatch, now_ms),
        Event::ClockStepped { by_ms } => {
            run::stopwatch_clock_stepped(&mut state.stopwatch, now_ms - by_ms, now_ms);
        }
    }
    effects(&before, state, now_ms)
}

/// What to do about a state that may have changed.
fn effects(before: &ClockState, after: &ClockState, now_ms: i64) -> Effects {
    Effects {
        persist: before != after,
        wake_at: crate::wake::next_wake_at(after, now_ms),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Stopwatch;
    use chrono::{FixedOffset, TimeZone};

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    fn at(h: u32, m: u32) -> i64 {
        utc()
            .with_ymd_and_hms(2026, 8, 5, h, m, 0)
            .unwrap()
            .timestamp_millis()
    }

    /// No city dataset: the tests that add one supply their own, so a call
    /// here would be a bug rather than a fallback.
    fn no_cities(_: &str) -> Option<WorldCity> {
        None
    }

    fn alarm(id: &str, time: &str) -> Alarm {
        Alarm {
            id: id.into(),
            time: time.into(),
            label: String::new(),
            days: vec![],
            enabled: true,
            fire_late: false,
            next_fire_at: None,
        }
    }

    #[test]
    fn setting_an_alarm_works_out_when_it_rings() {
        let mut s = ClockState::default();
        let fx = apply(
            &mut s,
            Command::SetAlarm(alarm("a", "07:00")),
            &utc(),
            no_cities,
            at(6, 0),
        );
        assert_eq!(s.alarms[0].next_fire_at, Some(at(7, 0)));
        assert!(fx.persist);
        assert_eq!(fx.wake_at, Some(at(7, 0)));
    }

    /// The `Power1` slot holds one wake, so it must be the earliest across
    /// everything - not the last thing the user touched.
    #[test]
    fn the_wake_follows_the_earliest_moment_not_the_latest_command() {
        let mut s = ClockState::default();
        apply(
            &mut s,
            Command::SetAlarm(alarm("morning", "07:00")),
            &utc(),
            no_cities,
            at(6, 0),
        );
        let fx = apply(
            &mut s,
            Command::SetAlarm(alarm("evening", "20:00")),
            &utc(),
            no_cities,
            at(6, 0),
        );
        assert_eq!(
            fx.wake_at,
            Some(at(7, 0)),
            "the morning alarm still holds it"
        );
    }

    #[test]
    fn setting_an_alarm_that_exists_replaces_it_rather_than_adding_a_second() {
        let mut s = ClockState::default();
        apply(
            &mut s,
            Command::SetAlarm(alarm("a", "07:00")),
            &utc(),
            no_cities,
            at(6, 0),
        );
        apply(
            &mut s,
            Command::SetAlarm(alarm("a", "08:00")),
            &utc(),
            no_cities,
            at(6, 0),
        );
        assert_eq!(s.alarms.len(), 1);
        assert_eq!(s.alarms[0].time, "08:00");
    }

    /// Re-enabling an alarm must not resurrect the moment that passed while it
    /// was off.
    #[test]
    fn re_enabling_an_alarm_looks_forward_rather_than_back() {
        let mut s = ClockState::default();
        apply(
            &mut s,
            Command::SetAlarm(alarm("a", "07:00")),
            &utc(),
            no_cities,
            at(6, 0),
        );
        apply(
            &mut s,
            Command::ToggleAlarm {
                id: "a".into(),
                enabled: false,
            },
            &utc(),
            no_cities,
            at(6, 30),
        );
        assert_eq!(s.alarms[0].next_fire_at, None);

        // Switched back on at nine, after the seven o'clock moment went by.
        apply(
            &mut s,
            Command::ToggleAlarm {
                id: "a".into(),
                enabled: true,
            },
            &utc(),
            no_cities,
            at(9, 0),
        );
        let due = s.alarms[0].next_fire_at.unwrap();
        assert!(due > at(9, 0), "the next ring is ahead, not this morning's");
    }

    #[test]
    fn a_command_that_changes_nothing_does_not_ask_for_a_write() {
        let mut s = ClockState::default();
        let fx = apply(
            &mut s,
            Command::DeleteAlarm { id: "nope".into() },
            &utc(),
            no_cities,
            at(6, 0),
        );
        assert!(!fx.persist);
    }

    #[test]
    fn cancelling_a_timer_drops_the_wake_it_held() {
        let mut s = ClockState::default();
        apply(
            &mut s,
            Command::TimerStart {
                id: "t".into(),
                duration_ms: 60_000,
            },
            &utc(),
            no_cities,
            at(6, 0),
        );
        let fx = apply(
            &mut s,
            Command::TimerCancel { id: "t".into() },
            &utc(),
            no_cities,
            at(6, 0),
        );
        assert_eq!(fx.wake_at, None);
        assert!(s.timers.is_empty());
    }

    #[test]
    fn pausing_a_timer_drops_its_wake_and_resuming_restores_one() {
        let mut s = ClockState::default();
        apply(
            &mut s,
            Command::TimerStart {
                id: "t".into(),
                duration_ms: 60_000,
            },
            &utc(),
            no_cities,
            at(6, 0),
        );
        let paused = apply(
            &mut s,
            Command::TimerSetPaused {
                id: "t".into(),
                paused: true,
            },
            &utc(),
            no_cities,
            at(6, 0),
        );
        assert_eq!(paused.wake_at, None, "a paused timer has no moment");
        let resumed = apply(
            &mut s,
            Command::TimerSetPaused {
                id: "t".into(),
                paused: false,
            },
            &utc(),
            no_cities,
            at(6, 1),
        );
        assert_eq!(resumed.wake_at, Some(at(6, 1) + 60_000));
    }

    #[test]
    fn a_city_is_not_added_twice() {
        let mut s = ClockState::default();
        let tokyo = |id: &str| {
            (id == "w").then(|| WorldCity {
                id: "w".into(),
                name: "Tokyo".into(),
                zone: "Asia/Tokyo".into(),
            })
        };
        apply(
            &mut s,
            Command::WorldAdd { id: "w".into() },
            &utc(),
            tokyo,
            0,
        );
        let fx = apply(
            &mut s,
            Command::WorldAdd { id: "w".into() },
            &utc(),
            tokyo,
            0,
        );
        assert_eq!(s.world.len(), 1);
        assert!(!fx.persist, "adding it again changed nothing");
    }

    /// Sleep is something that happened, not something the app asked for, and
    /// it reaches the state the same way.
    #[test]
    fn a_suspend_stops_the_stopwatch_counting() {
        let mut s = ClockState::default();
        apply(&mut s, Command::StopwatchStart, &utc(), no_cities, 0);
        let fx = observe(&mut s, Event::Suspending, 60_000);
        assert!(fx.persist);
        assert_eq!(s.stopwatch.accumulated_ms, 60_000);
        assert_eq!(s.stopwatch.started_at, None);
        assert!(s.stopwatch.running);

        observe(&mut s, Event::Resumed, 8 * 3_600_000);
        assert_eq!(
            run::stopwatch_elapsed(&s.stopwatch, 8 * 3_600_000 + 1_000),
            61_000,
            "the night is not counted"
        );
    }

    #[test]
    fn a_clock_step_leaves_the_stopwatch_reading_where_it_was() {
        let mut s = ClockState {
            stopwatch: Stopwatch {
                running: true,
                started_at: Some(0),
                accumulated_ms: 0,
                laps: vec![],
            },
            ..ClockState::default()
        };
        observe(
            &mut s,
            Event::ClockStepped { by_ms: 300_000 },
            300_000 + 30_000,
        );
        assert_eq!(
            run::stopwatch_elapsed(&s.stopwatch, 300_000 + 30_000),
            30_000
        );
    }

    #[test]
    fn a_focus_session_starts_and_holds_the_wake_for_its_phase_end() {
        let mut s = ClockState::default();
        let fx = apply(
            &mut s,
            Command::FocusStart {
                held: vec!["notifications".into()],
            },
            &utc(),
            no_cities,
            at(9, 0),
        );
        assert_eq!(fx.wake_at, Some(at(9, 25)));
        assert_eq!(
            s.focus.as_ref().unwrap().held,
            vec!["notifications".to_string()]
        );

        let ended = apply(&mut s, Command::FocusEnd, &utc(), no_cities, at(9, 10));
        assert!(s.focus.is_none());
        assert_eq!(ended.wake_at, None);
    }
}
