//! Scheduling a wake-up that survives suspend.
//!
//! Two apps rest entirely on this working - the clock's alarms and the calendar's
//! reminders - so it is proven on its own before either is built on top of it.
//!
//! The mechanism is a `CLOCK_REALTIME_ALARM` timerfd. Not
//! `/sys/class/rtc/rtc0/wakealarm`: that is root-owned, and it is the path
//! upstream explicitly declined to take.
//!
//! **The capability is probed, never assumed.** `CAP_WAKE_ALARM` reaches a
//! local-seat session by default from systemd v254, but a `systemd --user` unit
//! needs an explicit `AmbientCapabilities=CAP_WAKE_ALARM`, an SSH session never
//! gets it, and v255 broke `AmbientCapabilities` outright. A version floor would
//! therefore be a guess that fails silently on the machines it is wrong about. So
//! the probe is the syscall itself: ask the kernel for an alarm timer and see
//! whether it says no. That cannot be wrong about the machine it is running on.
//!
//! When the capability is absent the daemon does NOT pretend. It falls back to a
//! plain `CLOCK_REALTIME` timer, which fires correctly while the machine is awake
//! and cannot wake it, and it says so - so a caller can tell the user their alarm
//! will not wake the machine instead of discovering it at the time it mattered.

use std::io;
use std::os::unix::io::{FromRawFd, OwnedFd, RawFd};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Whether this process can arm a timer that wakes a suspended machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeCapability {
    /// `CLOCK_REALTIME_ALARM` was accepted: an armed alarm wakes the machine.
    WakesMachine,
    /// The kernel refused the alarm clock. Timers still fire while awake; they
    /// will not bring the machine back from suspend.
    AwakeOnly,
}

impl WakeCapability {
    /// Plain-language state for a caller to show, rather than a bare bool.
    pub fn describe(self) -> &'static str {
        match self {
            Self::WakesMachine => "alarms can wake this machine",
            Self::AwakeOnly => "alarms will not wake this machine",
        }
    }
}

/// Errors from scheduling a wake.
#[derive(Debug, thiserror::Error)]
pub enum WakeError {
    #[error("wake time is in the past")]
    InThePast,
    #[error("cannot create timer: {0}")]
    Timer(io::Error),
    #[error("cannot arm timer: {0}")]
    Arm(io::Error),
}

/// Try to create an alarm-clock timerfd, returning the raw fd.
///
/// Separated so the probe and the real arming take exactly the same path - a
/// probe that tests something other than what runs is a probe of the wrong thing.
fn create_timerfd(alarm: bool) -> io::Result<RawFd> {
    // 0x0008 CLOCK_REALTIME_ALARM, 0 CLOCK_REALTIME. TFD_CLOEXEC | TFD_NONBLOCK.
    let clock = if alarm { libc::CLOCK_REALTIME_ALARM } else { libc::CLOCK_REALTIME };
    // SAFETY: a plain syscall with constant flags; the fd is owned by the caller.
    let fd = unsafe { libc::timerfd_create(clock, libc::TFD_CLOEXEC | libc::TFD_NONBLOCK) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(fd)
}

/// Ask the kernel, at startup, whether this process may arm a waking alarm.
///
/// Creates and immediately drops an alarm timerfd. `EPERM` is the answer that
/// matters and the only one treated as "no capability"; any other failure is also
/// reported as awake-only, because a timer we could not create cannot wake
/// anything either way.
pub fn probe() -> WakeCapability {
    match create_timerfd(true) {
        Ok(fd) => {
            // SAFETY: fd came from timerfd_create above and is not used again.
            drop(unsafe { OwnedFd::from_raw_fd(fd) });
            WakeCapability::WakesMachine
        }
        Err(_) => WakeCapability::AwakeOnly,
    }
}

/// An armed wake. Dropping it disarms, because the fd is closed.
#[derive(Debug)]
pub struct ArmedWake {
    _fd: OwnedFd,
    /// What this alarm will actually do, so a caller never has to re-probe to
    /// know whether it can be relied on across a suspend.
    pub capability: WakeCapability,
}

/// Arm a wake for an absolute wall-clock time.
///
/// Absolute rather than a duration on purpose: the machine may suspend between
/// the call and the deadline, and a relative timer that stops counting while
/// suspended is exactly the failure this exists to avoid.
pub fn arm_at(unix_seconds: u64, capability: WakeCapability) -> Result<ArmedWake, WakeError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();
    if unix_seconds <= now {
        return Err(WakeError::InThePast);
    }
    let fd = create_timerfd(capability == WakeCapability::WakesMachine).map_err(WakeError::Timer)?;
    // SAFETY: fd came from timerfd_create and ownership moves here, so it is
    // closed exactly once even if arming below fails.
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    let spec = libc::itimerspec {
        it_interval: libc::timespec { tv_sec: 0, tv_nsec: 0 },
        it_value: libc::timespec { tv_sec: unix_seconds as libc::time_t, tv_nsec: 0 },
    };
    // SAFETY: `fd` is open, `spec` is fully initialised, and the old-value out
    // parameter is allowed to be null.
    let rc = unsafe {
        libc::timerfd_settime(fd, libc::TFD_TIMER_ABSTIME, &spec, std::ptr::null_mut())
    };
    if rc < 0 {
        return Err(WakeError::Arm(io::Error::last_os_error()));
    }
    Ok(ArmedWake { _fd: owned, capability })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probing_answers_without_panicking_either_way() {
        // Both answers are legitimate: a local seat gets the capability, a build
        // container does not. What matters is that asking is safe and total.
        let cap = probe();
        assert!(matches!(
            cap,
            WakeCapability::WakesMachine | WakeCapability::AwakeOnly
        ));
        assert!(!cap.describe().is_empty());
    }

    #[test]
    fn the_two_states_do_not_describe_themselves_the_same_way() {
        // The fallback exists to be TOLD to the user; if both read alike, the
        // honest degradation is invisible, which is the failure being avoided.
        assert_ne!(
            WakeCapability::WakesMachine.describe(),
            WakeCapability::AwakeOnly.describe()
        );
    }

    #[test]
    fn a_wake_in_the_past_is_refused() {
        let err = arm_at(1, WakeCapability::AwakeOnly).unwrap_err();
        assert!(matches!(err, WakeError::InThePast));
    }

    #[test]
    fn a_future_wake_arms_under_whatever_capability_this_machine_has() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        // Plain-timer path, which every machine can do, so the test asserts the
        // arming itself rather than the privilege.
        let armed = arm_at(now + 3600, WakeCapability::AwakeOnly).expect("arm a plain timer");
        assert_eq!(armed.capability, WakeCapability::AwakeOnly);
    }
}
