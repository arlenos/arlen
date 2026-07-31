// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! When a failed bridge should be tried again, and when it must not be.
//!
//! `bridge-architecture.md` §5 gives every bridge the same restart behaviour so
//! none has to invent it: bounded exponential backoff for transient errors, and
//! **no silent auto-restart of a hard failure**. The second half is the
//! deliberate part, taken from Kafka Connect - a connector that restarts itself
//! forever on a bad credential hides the breakage, and the user sees a bridge
//! that is "running" and silently doing nothing. A hard failure stops and says
//! why; restarting it is a decision someone makes.
//!
//! Pure, and the schedule is deterministic: no jitter. Jitter earns its keep
//! when many clients retry a shared upstream in lockstep, and bridges are
//! independent processes failing at independent times. Adding randomness now
//! would only make the schedule untestable.

/// Why an attempt failed, in the only distinction the restart rule needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// Something that plausibly succeeds on its own later: the network was
    /// down, the upstream returned a 5xx, an access token had expired. Worth
    /// retrying on a schedule.
    Transient,
    /// Something that will fail identically until a human changes it: a
    /// rejected credential, a malformed mapping, a revoked grant. Retrying is
    /// noise that hides the cause.
    Hard,
}

/// First delay after a transient failure.
const BASE_DELAY_MICROS: i64 = 1_000_000;
/// Ceiling on the delay. Past this, waiting longer stops buying anything and
/// only makes recovery from a long outage feel broken.
const MAX_DELAY_MICROS: i64 = 5 * 60 * 1_000_000;
/// How many transient attempts before the host stops on its own.
///
/// Bounded rather than infinite: an upstream that has refused for this long
/// with a "transient" error is not, in practice, having a moment. Stopping puts
/// it in front of the user instead of leaving a process retrying into the void.
pub const MAX_TRANSIENT_ATTEMPTS: u32 = 10;

/// What the host should do after a failed attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDecision {
    /// Wait this many microseconds, then try again.
    RetryAfterMicros(i64),
    /// Do not retry. The bridge stops and reports why; restarting is explicit.
    GiveUp,
}

/// How long to wait before attempt number `attempt` (1 = the first retry,
/// after the initial try failed).
///
/// Doubling from one second, capped. `attempt` 0 is treated as 1 rather than
/// yielding a zero delay, because a caller that forgot the numbering should not
/// get a hot loop out of it.
pub fn backoff_delay_micros(attempt: u32) -> i64 {
    let step = attempt.max(1) - 1;
    // Saturating on both sides: a large attempt count must clamp to the ceiling
    // rather than overflow into a small or negative delay.
    let factor = 2i64.checked_pow(step.min(62)).unwrap_or(i64::MAX);
    BASE_DELAY_MICROS
        .saturating_mul(factor)
        .min(MAX_DELAY_MICROS)
}

/// Whether to retry, given what went wrong and how many attempts have failed in
/// a row.
///
/// A hard failure never retries, at any count, including the first: the point
/// is that the cause is not time.
///
/// The count is CONSECUTIVE, and the caller owns resetting it. A success clears
/// it, and so should an external signal that the world changed - the network
/// coming back, the machine resuming. That matters more than it looks: ten
/// doublings capped at five minutes is about thirteen minutes of trying, which
/// a closed laptop passes without noticing, and a bridge that gave up while
/// suspended and stayed given-up would be a worse failure than the one it was
/// avoiding. Bounding the attempts is right; treating a suspend as thirteen
/// minutes of a refusing upstream is not.
pub fn retry_decision(kind: FailureKind, consecutive_failures: u32) -> RetryDecision {
    match kind {
        FailureKind::Hard => RetryDecision::GiveUp,
        FailureKind::Transient if consecutive_failures >= MAX_TRANSIENT_ATTEMPTS => RetryDecision::GiveUp,
        FailureKind::Transient => {
            RetryDecision::RetryAfterMicros(backoff_delay_micros(consecutive_failures))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_delay_doubles_from_one_second() {
        assert_eq!(backoff_delay_micros(1), 1_000_000);
        assert_eq!(backoff_delay_micros(2), 2_000_000);
        assert_eq!(backoff_delay_micros(3), 4_000_000);
        assert_eq!(backoff_delay_micros(4), 8_000_000);
    }

    #[test]
    fn the_delay_stops_growing_at_the_ceiling() {
        assert_eq!(backoff_delay_micros(20), MAX_DELAY_MICROS);
        // And nothing overflows into a small or negative wait at absurd counts,
        // which would turn a backoff into a hot loop.
        assert_eq!(backoff_delay_micros(u32::MAX), MAX_DELAY_MICROS);
    }

    /// A caller that passes 0 means "the first retry", not "immediately".
    #[test]
    fn attempt_zero_waits_rather_than_looping_hot() {
        assert_eq!(backoff_delay_micros(0), backoff_delay_micros(1));
        assert!(backoff_delay_micros(0) > 0);
    }

    /// The rule the whole module exists for. A hard failure is not retried at
    /// any count, including before any attempt has been made, because what is
    /// wrong is not the passage of time.
    #[test]
    fn a_hard_failure_is_never_retried() {
        for attempts in [0, 1, 5, MAX_TRANSIENT_ATTEMPTS, u32::MAX] {
            assert_eq!(
                retry_decision(FailureKind::Hard, attempts),
                RetryDecision::GiveUp,
                "at {attempts} attempts"
            );
        }
    }

    #[test]
    fn a_transient_failure_retries_on_the_schedule_until_the_bound() {
        assert_eq!(
            retry_decision(FailureKind::Transient, 1),
            RetryDecision::RetryAfterMicros(1_000_000)
        );
        assert_eq!(
            retry_decision(FailureKind::Transient, 3),
            RetryDecision::RetryAfterMicros(4_000_000)
        );
        // The bound is inclusive: having failed that many times, it stops.
        assert_eq!(
            retry_decision(FailureKind::Transient, MAX_TRANSIENT_ATTEMPTS),
            RetryDecision::GiveUp
        );
        assert_eq!(
            retry_decision(FailureKind::Transient, MAX_TRANSIENT_ATTEMPTS - 1),
            RetryDecision::RetryAfterMicros(backoff_delay_micros(MAX_TRANSIENT_ATTEMPTS - 1))
        );
    }
}
