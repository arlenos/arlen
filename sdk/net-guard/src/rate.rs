//! A token-bucket rate limiter for the egress authoriser (CONN-R3, closes part of
//! audit GAP-14): a per-connection cap (~60/min, Envoy-style) so an allowlisted
//! destination cannot be hammered through the one egress choke point. The allowlist
//! says WHERE a confined process may talk; this caps HOW FAST.
//!
//! Pure: the caller passes the current `Instant`, so the refill is deterministic and
//! unit-testable without sleeping. One limiter governs one connection's egress; the
//! proxy holds it behind its own lock.

use std::time::Instant;

/// A token bucket: `capacity` burst tokens that refill at `refill_per_sec`. Each
/// permitted request spends one token; when the bucket is empty, requests are refused
/// until tokens refill. Starting full lets a connection burst up to `capacity`
/// immediately, then settle to the sustained `refill_per_sec` rate.
#[derive(Debug, Clone)]
pub struct RateLimiter {
    capacity: f64,
    refill_per_sec: f64,
    tokens: f64,
    last: Instant,
}

impl RateLimiter {
    /// A limiter holding `capacity` burst tokens, refilling `refill_per_sec` tokens a
    /// second, full as of `now`. A `capacity` of 0 refuses everything (a fail-closed
    /// "no egress" cap); a `refill_per_sec` of 0 never refills (a one-shot burst).
    pub fn new(capacity: u32, refill_per_sec: f64, now: Instant) -> Self {
        let capacity = f64::from(capacity);
        Self {
            capacity,
            refill_per_sec: refill_per_sec.max(0.0),
            tokens: capacity,
            last: now,
        }
    }

    /// The conventional egress cap: `rpm` requests a minute, with `rpm` burst tokens,
    /// refilling at `rpm/60` a second (the CONN-R3 ~60/min default).
    pub fn per_minute(rpm: u32, now: Instant) -> Self {
        Self::new(rpm, f64::from(rpm) / 60.0, now)
    }

    /// Try to spend one token as of `now`. Refills first (capped at `capacity`), then
    /// permits and spends a token when at least one is available, else refuses without
    /// spending. `now` going backwards (a non-monotonic caller) only withholds refill,
    /// never grants extra, so the cap can never be widened by a clock glitch.
    pub fn try_acquire(&mut self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_per_sec).min(self.capacity);
        self.last = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn permits_up_to_capacity_then_refuses() {
        let t0 = Instant::now();
        let mut rl = RateLimiter::new(3, 0.0, t0);
        // Three burst tokens, then dry (no refill).
        assert!(rl.try_acquire(t0));
        assert!(rl.try_acquire(t0));
        assert!(rl.try_acquire(t0));
        assert!(!rl.try_acquire(t0), "the bucket is empty after capacity acquires");
    }

    #[test]
    fn refills_over_time_up_to_capacity() {
        let t0 = Instant::now();
        let mut rl = RateLimiter::new(2, 1.0, t0); // 1 token/sec, burst 2
        assert!(rl.try_acquire(t0));
        assert!(rl.try_acquire(t0));
        assert!(!rl.try_acquire(t0));
        // One second later, one token has refilled.
        let t1 = t0 + Duration::from_secs(1);
        assert!(rl.try_acquire(t1));
        assert!(!rl.try_acquire(t1));
        // A long idle refills only to capacity, never beyond (no unbounded credit).
        let t2 = t1 + Duration::from_secs(3600);
        assert!(rl.try_acquire(t2));
        assert!(rl.try_acquire(t2));
        assert!(!rl.try_acquire(t2), "refill is capped at capacity");
    }

    #[test]
    fn a_backwards_clock_never_grants_extra() {
        let t1 = Instant::now() + Duration::from_secs(10);
        let mut rl = RateLimiter::new(1, 1.0, t1);
        assert!(rl.try_acquire(t1));
        // `now` before `last`: saturating_duration_since is zero, so no refill.
        let earlier = t1 - Duration::from_secs(5);
        assert!(!rl.try_acquire(earlier), "a backwards clock must not refill");
    }

    #[test]
    fn per_minute_bursts_then_sustains() {
        let t0 = Instant::now();
        let mut rl = RateLimiter::per_minute(60, t0); // 60 burst, 1/sec sustained
        for _ in 0..60 {
            assert!(rl.try_acquire(t0));
        }
        assert!(!rl.try_acquire(t0), "the minute's burst is spent");
        // One second refills exactly one token at the sustained rate.
        let t1 = t0 + Duration::from_secs(1);
        assert!(rl.try_acquire(t1));
        assert!(!rl.try_acquire(t1));
    }

    #[test]
    fn zero_capacity_refuses_everything() {
        let t0 = Instant::now();
        let mut rl = RateLimiter::new(0, 10.0, t0);
        assert!(!rl.try_acquire(t0));
        assert!(!rl.try_acquire(t0 + Duration::from_secs(60)), "0 capacity caps at 0");
    }
}
