//! Starting, pausing and resuming the things that run: timers and the stopwatch.
//!
//! **The two behave differently across a closed lid, and that is deliberate.**
//! A countdown timer is a promise about wall-clock time - twenty-five minutes
//! from now means twenty-five minutes from now, whether or not the machine
//! slept through some of it - so a timer is anchored to `ends_at` and suspend
//! needs no handling at all. A stopwatch measures *use*, and counting the eight
//! hours a laptop spent shut would be nonsense, so it stops when the machine
//! does.
//!
//! **That stop is the daemon's job, not the kernel's, and the mechanism is
//! chosen so the two suspend types cannot disagree.** The obvious
//! implementation - measure with `CLOCK_MONOTONIC` and let the kernel do the
//! pausing - gives different answers on different machines: monotonic time
//! excludes suspend-to-RAM but **keeps advancing under s2idle**, which is what
//! most 2024-and-later laptops actually enter when the lid closes. The same
//! code would then read "stopwatch paused" on one laptop and "stopwatch ran all
//! night" on another, with nothing in the interface explaining why.
//!
//! So nothing here asks a clock to pause. The stopwatch is an explicit
//! subtraction: the daemon folds the current run into the total when the
//! machine goes to sleep and opens a new one when it wakes. **That is
//! independent of which suspend type the machine used** - it is the same
//! arithmetic for s2idle, suspend-to-RAM and, for that matter, a laptop carried
//! between two of them - because no kernel clock behaviour is being relied on.
//!
//! **What it does depend on, stated because it is now the weak point:** the
//! daemon has to be told. The fold happens on logind's `PrepareForSleep`, and a
//! sleep that arrives without one counts as running time. That is a signal
//! question rather than a clock question, and it fails the same way on both
//! suspend types rather than silently differing between them.
//!
//! **And the trade this choice makes:** anchors are wall-clock instants, so a
//! clock step - NTP correcting a drifting machine, or someone setting the time -
//! would move the reading. [`stopwatch_clock_stepped`] is how the daemon absorbs
//! that; monotonic time would not have needed it, which is the price of not
//! depending on monotonic time's suspend behaviour.

use crate::state::{Stopwatch, Timer};

/// Start a countdown of `duration_ms` from now.
pub fn timer_start(id: String, duration_ms: i64, now_ms: i64) -> Timer {
    Timer {
        id,
        duration_ms,
        ends_at: Some(now_ms + duration_ms),
        remaining_ms: None,
        paused: false,
    }
}

/// Pause a running timer, keeping what was left of it.
///
/// The anchor goes and a snapshot replaces it, because a paused timer has no
/// end instant - keeping both would let them disagree the moment it resumes.
/// Already-paused is a no-op rather than an error: the caller pressing pause
/// twice means the same thing both times.
pub fn timer_pause(timer: &mut Timer, now_ms: i64) {
    let Some(ends_at) = timer.ends_at else { return };
    timer.remaining_ms = Some((ends_at - now_ms).max(0));
    timer.ends_at = None;
    timer.paused = true;
}

/// Resume a paused timer, from what was left of it.
pub fn timer_resume(timer: &mut Timer, now_ms: i64) {
    let Some(remaining) = timer.remaining_ms else {
        return;
    };
    timer.ends_at = Some(now_ms + remaining);
    timer.remaining_ms = None;
    timer.paused = false;
}

/// Whether a running timer has reached its end.
///
/// A paused timer never has: it is holding, not finished, however long it has
/// been held.
pub fn timer_elapsed(timer: &Timer, now_ms: i64) -> bool {
    timer.ends_at.is_some_and(|ends_at| now_ms >= ends_at)
}

/// Start or resume the stopwatch.
pub fn stopwatch_start(watch: &mut Stopwatch, now_ms: i64) {
    if watch.running && watch.started_at.is_some() {
        return;
    }
    watch.running = true;
    watch.started_at = Some(now_ms);
}

/// Pause the stopwatch, folding the current run into the total.
pub fn stopwatch_pause(watch: &mut Stopwatch, now_ms: i64) {
    fold_run(watch, now_ms);
    watch.running = false;
}

/// Stop counting because the machine is going to sleep.
///
/// The run is folded exactly as a pause folds it, but `running` stays true: the
/// user did not stop the stopwatch, the machine did, and it picks up again on
/// [`stopwatch_resumed`] without them touching anything. Between the two there
/// is nothing to render, because there is nobody at the screen.
pub fn stopwatch_suspended(watch: &mut Stopwatch, now_ms: i64) {
    if watch.running {
        fold_run(watch, now_ms);
    }
}

/// Start counting again after the machine wakes.
pub fn stopwatch_resumed(watch: &mut Stopwatch, now_ms: i64) {
    if watch.running && watch.started_at.is_none() {
        watch.started_at = Some(now_ms);
    }
}

/// Absorb a wall-clock step so the elapsed reading does not move.
///
/// The anchors here are epoch instants, which is what lets the app derive a
/// display without the daemon sending counters - but it means a correction to
/// the system clock would otherwise add or remove that correction from a
/// running stopwatch. NTP disciplining a machine that has been asleep is the
/// ordinary case, and someone setting the clock by hand is the loud one.
///
/// Shifting the anchor by the same step leaves the elapsed time exactly as it
/// was, which is what a person watching a stopwatch expects to see when the
/// clock beside it jumps.
pub fn stopwatch_clock_stepped(watch: &mut Stopwatch, old_now_ms: i64, new_now_ms: i64) {
    if let Some(started) = watch.started_at.as_mut() {
        *started += new_now_ms - old_now_ms;
    }
}

/// Take a lap: record the total so far, without interrupting the run.
pub fn stopwatch_lap(watch: &mut Stopwatch, now_ms: i64) {
    watch.laps.push(stopwatch_elapsed(watch, now_ms));
}

/// Back to zero, running or not.
pub fn stopwatch_reset(watch: &mut Stopwatch) {
    *watch = Stopwatch::default();
}

/// How long the stopwatch has been running, in ms.
///
/// The completed runs plus the current one, which is what makes a suspended
/// stopwatch read correctly: with no current run, the total simply stops
/// growing.
pub fn stopwatch_elapsed(watch: &Stopwatch, now_ms: i64) -> i64 {
    let current = watch.started_at.map_or(0, |at| (now_ms - at).max(0));
    watch.accumulated_ms + current
}

/// Move the current run into the total and end it.
fn fold_run(watch: &mut Stopwatch, now_ms: i64) {
    if let Some(started) = watch.started_at.take() {
        watch.accumulated_ms += (now_ms - started).max(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    #[test]
    fn a_timer_ends_a_duration_from_when_it_started() {
        let t = timer_start("t".into(), 25 * MIN, 1_000);
        assert_eq!(t.ends_at, Some(1_000 + 25 * MIN));
        assert_eq!(t.remaining_ms, None);
        assert!(!t.paused);
    }

    #[test]
    fn pausing_swaps_the_anchor_for_what_is_left() {
        let mut t = timer_start("t".into(), 25 * MIN, 0);
        timer_pause(&mut t, 10 * MIN);
        assert_eq!(t.ends_at, None);
        assert_eq!(t.remaining_ms, Some(15 * MIN));
        assert!(t.paused);
    }

    /// Held is not finished, however long it is held for.
    #[test]
    fn a_paused_timer_does_not_run_down_while_paused() {
        let mut t = timer_start("t".into(), 5 * MIN, 0);
        timer_pause(&mut t, MIN);
        assert!(!timer_elapsed(&t, 10 * 60 * MIN));
        timer_resume(&mut t, 10 * 60 * MIN);
        assert_eq!(t.ends_at, Some(10 * 60 * MIN + 4 * MIN));
    }

    #[test]
    fn a_timer_paused_past_its_end_has_nothing_left_rather_than_less_than_nothing() {
        let mut t = timer_start("t".into(), MIN, 0);
        timer_pause(&mut t, 5 * MIN);
        assert_eq!(t.remaining_ms, Some(0));
    }

    #[test]
    fn pressing_pause_twice_means_the_same_as_once() {
        let mut t = timer_start("t".into(), 5 * MIN, 0);
        timer_pause(&mut t, MIN);
        let after_first = t.clone();
        timer_pause(&mut t, 3 * MIN);
        assert_eq!(t, after_first);
    }

    #[test]
    fn a_timer_is_elapsed_at_its_end_instant() {
        let t = timer_start("t".into(), MIN, 0);
        assert!(!timer_elapsed(&t, MIN - 1));
        assert!(timer_elapsed(&t, MIN));
    }

    #[test]
    fn the_stopwatch_accumulates_across_pauses() {
        let mut w = Stopwatch::default();
        stopwatch_start(&mut w, 0);
        stopwatch_pause(&mut w, 30_000);
        assert_eq!(stopwatch_elapsed(&w, 90_000), 30_000);
        stopwatch_start(&mut w, 100_000);
        assert_eq!(stopwatch_elapsed(&w, 110_000), 40_000);
    }

    /// The one this module exists for: a laptop shut for eight hours must not
    /// add eight hours to a stopwatch.
    #[test]
    fn a_suspended_machine_does_not_count_towards_the_stopwatch() {
        let mut w = Stopwatch::default();
        stopwatch_start(&mut w, 0);
        let sleep_at = 5 * MIN;
        stopwatch_suspended(&mut w, sleep_at);
        let wake_at = sleep_at + 8 * 60 * MIN;
        stopwatch_resumed(&mut w, wake_at);
        // Five minutes before, one minute after, and nothing for the night.
        assert_eq!(stopwatch_elapsed(&w, wake_at + MIN), 6 * MIN);
        assert!(w.running, "the user never stopped it, the machine did");
    }

    /// A timer is the opposite case on purpose: it is a promise about wall
    /// clock time, so sleeping through part of it changes nothing.
    #[test]
    fn a_suspended_machine_does_not_extend_a_timer() {
        let t = timer_start("t".into(), 25 * MIN, 0);
        // Woken an hour later: long finished, as the person expects.
        assert!(timer_elapsed(&t, 60 * MIN));
    }

    #[test]
    fn a_stopwatch_paused_by_the_user_stays_paused_through_a_suspend() {
        let mut w = Stopwatch::default();
        stopwatch_start(&mut w, 0);
        stopwatch_pause(&mut w, MIN);
        stopwatch_suspended(&mut w, 2 * MIN);
        stopwatch_resumed(&mut w, 3 * MIN);
        assert!(!w.running);
        assert_eq!(w.started_at, None);
        assert_eq!(stopwatch_elapsed(&w, 10 * MIN), MIN);
    }

    #[test]
    fn a_lap_records_the_total_without_stopping_the_run() {
        let mut w = Stopwatch::default();
        stopwatch_start(&mut w, 0);
        stopwatch_lap(&mut w, 10_000);
        stopwatch_lap(&mut w, 25_000);
        assert_eq!(w.laps, vec![10_000, 25_000]);
        assert!(w.running);
        assert_eq!(stopwatch_elapsed(&w, 30_000), 30_000);
    }

    #[test]
    fn reset_clears_the_laps_and_the_total() {
        let mut w = Stopwatch::default();
        stopwatch_start(&mut w, 0);
        stopwatch_lap(&mut w, 5_000);
        stopwatch_reset(&mut w);
        assert_eq!(w, Stopwatch::default());
        assert_eq!(stopwatch_elapsed(&w, 99_000), 0);
    }

    /// Pressing start on a running stopwatch must not restart its current run,
    /// which would silently lose the time since it began.
    #[test]
    fn starting_a_running_stopwatch_changes_nothing() {
        let mut w = Stopwatch::default();
        stopwatch_start(&mut w, 0);
        stopwatch_start(&mut w, 30_000);
        assert_eq!(w.started_at, Some(0));
        assert_eq!(stopwatch_elapsed(&w, 60_000), 60_000);
    }

    /// The price of not depending on monotonic time: a clock correction must
    /// not move the reading.
    #[test]
    fn a_clock_step_does_not_change_how_long_the_stopwatch_has_run() {
        let mut w = Stopwatch::default();
        stopwatch_start(&mut w, 0);
        let before = stopwatch_elapsed(&w, 30_000);
        // NTP pulls the machine forward five minutes.
        stopwatch_clock_stepped(&mut w, 30_000, 30_000 + 5 * MIN);
        assert_eq!(stopwatch_elapsed(&w, 30_000 + 5 * MIN), before);
        // And backwards, which is the direction that would otherwise read as a
        // stopwatch running in reverse.
        stopwatch_clock_stepped(&mut w, 30_000 + 5 * MIN, 30_000);
        assert_eq!(stopwatch_elapsed(&w, 30_000), before);
    }

    #[test]
    fn a_clock_step_while_paused_changes_nothing() {
        let mut w = Stopwatch::default();
        stopwatch_start(&mut w, 0);
        stopwatch_pause(&mut w, MIN);
        let before = w.clone();
        stopwatch_clock_stepped(&mut w, MIN, MIN + 60 * MIN);
        assert_eq!(w, before);
    }

    /// A wake with no matching suspend must not restart the run either - a
    /// resume event the daemon did not expect should be inert, not destructive.
    #[test]
    fn a_resume_without_a_suspend_is_inert() {
        let mut w = Stopwatch::default();
        stopwatch_start(&mut w, 0);
        stopwatch_resumed(&mut w, 30_000);
        assert_eq!(w.started_at, Some(0));
    }
}
