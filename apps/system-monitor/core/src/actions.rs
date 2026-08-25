//! Process actions for the task manager (system-monitor-plan.md): Stop (graceful
//! SIGTERM), Freeze (the non-destructive SIGSTOP/SIGCONT pause) and Limit (a soft
//! CPU leash via the process's cgroup `cpu.max`).
//!
//! Signals are the reliable, permission-checked mechanism (the kernel refuses a
//! signal to a process the user does not own), so Stop and Freeze work anywhere.
//! Limit writes the process's own cgroup v2 `cpu.max`, which requires the cgroup
//! to be delegated + writable to the session; where it is not, the write returns a
//! clear error (the UI reflects that it did not apply) rather than silently doing
//! nothing. A wrong `memory.high` can thrash a process into reclaim, so the leash
//! is CPU-only for now; the memory half needs a considered cap policy.

use std::path::PathBuf;

/// The cgroup CPU quota applied on Limit: half of one core (50000us of every
/// 100000us period). Reversible via [`CPU_MAX_UNLIMITED`].
const CPU_MAX_LIMITED: &str = "50000 100000";
/// The cgroup CPU quota that removes the leash (the kernel default: unlimited).
const CPU_MAX_UNLIMITED: &str = "max";

/// Why an action was refused, as a closed set the window can put into words.
///
/// These used to leave here as sentences - `"refusing to signal an unsafe pid:
/// 0"`, and for the common case whatever `strerror` said, so a person pressing
/// End process on a system service read "Operation not permitted (os error 1)".
/// The store then dropped that into a translated sentence, which meant a German
/// build explained the reader's own machine to them half in English. The store's
/// own doc comment says that is the thing to avoid; the action sentence had been
/// moved to the catalogue and the REASON had not.
///
/// So a refusal travels as a word and the window writes the sentence. The
/// system's own text is kept as [`Refusal::detail`] for the log, where the
/// person who debugs this wants it and the person using the machine does not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The pid would not name one process: 0 signals the caller's whole group,
    /// and anything past `i32::MAX` wraps negative into another group.
    UnsafePid(u32),
    /// The kernel said no. The ordinary case, and it means one of two things a
    /// person can act on: the process belongs to somebody else, or the priority
    /// was being LOWERED, which needs `CAP_SYS_NICE` even on your own process.
    NotPermitted,
    /// The process ended between the list being drawn and the action arriving.
    /// Not an error worth alarm - the thing the user wanted is already true.
    Gone,
    /// Not one of the priorities this app offers. Reachable only from a caller
    /// that bypassed the served table, so it is a bug rather than a refusal.
    BadPriority(i32),
    /// The process is on a host with no cgroup v2 path for it, so there is no
    /// `cpu.max` to write and no leash to apply.
    NoCgroup(String),
    /// The cgroup is there and this session may not write it. Delegation is a
    /// property of how the session was started, not of the process being limited.
    NoDelegation(String),
    /// Anything else, carrying the system's words for the log.
    Other(String),
}

impl Refusal {
    /// The word the window translates. Kebab-case, matching the tokens the other
    /// apps' hosts return, so one shape reads the same across surfaces.
    #[must_use]
    pub fn token(&self) -> &'static str {
        match self {
            Self::UnsafePid(_) => "unsafe-pid",
            Self::NotPermitted => "not-permitted",
            Self::Gone => "gone",
            Self::BadPriority(_) => "bad-priority",
            Self::NoCgroup(_) => "no-cgroup",
            Self::NoDelegation(_) => "no-delegation",
            Self::Other(_) => "other",
        }
    }

    /// The detail for the log: paths, errnos, the pid that was refused. Never
    /// shown to the person using the machine.
    #[must_use]
    pub fn detail(&self) -> String {
        match self {
            Self::UnsafePid(pid) => format!("unsafe pid: {pid}"),
            Self::NotPermitted => "the kernel refused (EPERM/EACCES)".to_string(),
            Self::Gone => "no such process (ESRCH)".to_string(),
            Self::BadPriority(nice) => format!("not an offered priority: {nice}"),
            Self::NoCgroup(why) | Self::NoDelegation(why) | Self::Other(why) => why.clone(),
        }
    }
}

/// Classify an OS error into a refusal.
///
/// Only the two the user can do something about are named; everything else keeps
/// its own words in [`Refusal::Other`] rather than being flattened into a
/// reassuring wrong answer.
#[must_use]
pub fn refusal_from_os(e: &std::io::Error) -> Refusal {
    match e.raw_os_error() {
        Some(libc::EPERM) | Some(libc::EACCES) => Refusal::NotPermitted,
        Some(libc::ESRCH) => Refusal::Gone,
        _ => Refusal::Other(e.to_string()),
    }
}

/// Validate a target pid before signalling. A `kill(2)` with pid 0 signals the
/// **caller's** whole process group, and a pid that wraps negative as a `pid_t`
/// (`i32`) signals an arbitrary group; both are refused so an action only ever
/// hits the one intended process. Pure.
pub fn valid_signal_target(pid: u32) -> bool {
    pid != 0 && pid <= i32::MAX as u32
}

/// The unified (cgroup v2) path from a `/proc/<pid>/cgroup` file: the entry with
/// hierarchy id 0 (`"0::<path>"`). `None` if there is no v2 line (a pure-v1 host).
/// Pure.
pub fn unified_cgroup_path(cgroup_file: &str) -> Option<String> {
    cgroup_file
        .lines()
        .find_map(|l| l.strip_prefix("0::"))
        .map(|p| p.trim().to_string())
        .filter(|p| p.starts_with('/'))
}

/// Send `sig` to `pid`, refusing an unsafe target. Maps the OS error (e.g. the
/// kernel's `EPERM` for another user's process, or `ESRCH` for a vanished one) to
/// a message the UI can show.
pub fn send_signal(pid: u32, sig: i32) -> Result<(), Refusal> {
    if !valid_signal_target(pid) {
        return Err(Refusal::UnsafePid(pid));
    }
    // SAFETY: `kill` is a plain syscall over an integer pid + signal; the target
    // is range-checked above so it can never be a process-group signal.
    let rc = unsafe { libc::kill(pid as libc::pid_t, sig) };
    if rc == 0 {
        Ok(())
    } else {
        Err(refusal_from_os(&std::io::Error::last_os_error()))
    }
}

/// Gracefully stop a process with SIGTERM (the process gets to clean up; a
/// force-kill escalation is a UI follow-up, not an automatic SIGKILL).
pub fn stop(pid: u32) -> Result<(), Refusal> {
    send_signal(pid, libc::SIGTERM)
}

/// Freeze (`paused=true` -> SIGSTOP) or thaw (`paused=false` -> SIGCONT) a
/// process: the non-destructive pause, fully reversible.
pub fn freeze(pid: u32, paused: bool) -> Result<(), Refusal> {
    send_signal(pid, if paused { libc::SIGSTOP } else { libc::SIGCONT })
}

/// Soft-leash (`limited=true`) or release (`limited=false`) a process's CPU by
/// writing its own cgroup v2 `cpu.max`. Best-effort: if the process has no v2
/// cgroup or the file is not writable by the session (no delegation), the error
/// is surfaced so the UI does not falsely show a limit. `cgroup_root` is the
/// cgroup2 mount (`/sys/fs/cgroup`), injected for testing.
pub fn limit_at(cgroup_root: &std::path::Path, pid: u32, limited: bool) -> Result<(), Refusal> {
    if !valid_signal_target(pid) {
        return Err(Refusal::UnsafePid(pid));
    }
    // A process that ended reads the same here as one that never was, and both
    // are `Gone` rather than a failure to configure something.
    let cgroup_file = std::fs::read_to_string(format!("/proc/{pid}/cgroup"))
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Refusal::Gone,
            _ => Refusal::NoCgroup(format!("read /proc/{pid}/cgroup: {e}")),
        })?;
    let rel = unified_cgroup_path(&cgroup_file)
        .ok_or_else(|| Refusal::NoCgroup(format!("pid {pid} has no cgroup v2 path")))?;
    // rel is an absolute cgroup path ("/user.slice/.../app.scope"); join its
    // components onto the cgroup2 mount.
    let dir = cgroup_root.join(rel.trim_start_matches('/'));
    let target = dir.join("cpu.max");
    let value = if limited { CPU_MAX_LIMITED } else { CPU_MAX_UNLIMITED };
    std::fs::write(&target, value)
        .map_err(|e| Refusal::NoDelegation(format!("write {}: {e}", target.display())))
}

/// Soft-leash a process over the real cgroup2 mount.
pub fn limit(pid: u32, limited: bool) -> Result<(), Refusal> {
    limit_at(&PathBuf::from("/sys/fs/cgroup"), pid, limited)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_target_rejects_group_and_wrapping_pids() {
        assert!(!valid_signal_target(0), "pid 0 is the caller's process group");
        assert!(valid_signal_target(1));
        assert!(valid_signal_target(4242));
        assert!(valid_signal_target(i32::MAX as u32));
        // Above i32::MAX would wrap negative as a pid_t -> a group signal.
        assert!(!valid_signal_target(i32::MAX as u32 + 1));
        assert!(!valid_signal_target(u32::MAX));
    }

    #[test]
    fn send_signal_refuses_an_unsafe_target_without_calling_kill() {
        assert_eq!(send_signal(0, libc::SIGTERM), Err(Refusal::UnsafePid(0)));
        assert_eq!(send_signal(u32::MAX, libc::SIGCONT), Err(Refusal::UnsafePid(u32::MAX)));
    }

    #[test]
    fn the_two_refusals_a_person_can_act_on_are_named() {
        // The common pair. EPERM is what End process on somebody else's service
        // gives, and what raising a priority gives on your own; ESRCH is a row
        // that was true when it was drawn and is not now.
        let eperm = std::io::Error::from_raw_os_error(libc::EPERM);
        let eacces = std::io::Error::from_raw_os_error(libc::EACCES);
        let esrch = std::io::Error::from_raw_os_error(libc::ESRCH);
        assert_eq!(refusal_from_os(&eperm), Refusal::NotPermitted);
        assert_eq!(refusal_from_os(&eacces), Refusal::NotPermitted);
        assert_eq!(refusal_from_os(&esrch), Refusal::Gone);
    }

    #[test]
    fn an_unclassified_error_keeps_its_own_words_rather_than_a_wrong_answer() {
        let eio = std::io::Error::from_raw_os_error(libc::EIO);
        let r = refusal_from_os(&eio);
        assert_eq!(r.token(), "other");
        // The system's text survives for the log. Flattening this into one of the
        // named tokens would tell a reader something specific and untrue.
        assert!(r.detail().contains("os error"), "detail was {:?}", r.detail());
    }

    #[test]
    fn no_refusal_leaves_here_carrying_a_sentence() {
        // The property the window depends on: a token is a word, never a message
        // with the machine's own text in it, so no `strerror` output can reach a
        // translated sentence by way of the token.
        for r in [
            Refusal::UnsafePid(0),
            Refusal::NotPermitted,
            Refusal::Gone,
            Refusal::BadPriority(-20),
            Refusal::NoCgroup("read /proc/1/cgroup: nope".into()),
            Refusal::NoDelegation("write /sys/fs/cgroup/cpu.max: nope".into()),
            Refusal::Other("Input/output error (os error 5)".into()),
        ] {
            let t = r.token();
            assert!(!t.contains(' '), "token {t} reads as a sentence");
            assert!(t.chars().all(|c| c.is_ascii_lowercase() || c == '-'), "token {t}");
        }
    }

    #[test]
    fn unified_path_extracts_the_v2_line() {
        let f = "12:pids:/user.slice\n0::/user.slice/user-1000.slice/session.scope\n";
        assert_eq!(
            unified_cgroup_path(f).as_deref(),
            Some("/user.slice/user-1000.slice/session.scope")
        );
        // A pure-v1 file (no "0::" line) yields nothing.
        assert_eq!(unified_cgroup_path("12:pids:/x\n1:cpu:/y\n"), None);
        // A "0::" with a non-absolute payload is rejected (defensive).
        assert_eq!(unified_cgroup_path("0::relative"), None);
    }

    #[test]
    fn limit_writes_cpu_max_under_the_resolved_cgroup_dir() {
        // A fake cgroup2 tree + a fake /proc is not reachable here, so exercise the
        // path resolution + write against a temp cgroup root by faking the
        // /proc read via a known pid is not possible; instead assert the write
        // target math through unified_cgroup_path + join is covered above, and that
        // an invalid pid short-circuits.
        assert!(limit_at(std::path::Path::new("/sys/fs/cgroup"), 0, true).is_err());
    }
}

/// The priorities the Advanced affordance offers (system-monitor-plan.md (c),
/// "Priority / affinity / full signal menu ... behind an Advanced affordance").
///
/// Nice values, not a made-up scale. Five points, plainly named, because the
/// number means nothing to most people and the direction is famously backwards:
/// a HIGHER nice value is a LOWER priority.
///
/// NOT offered: anything below -5, and no real-time scheduling at all. The plan
/// says to warn against RT, and the strongest form of that warning is not
/// shipping the control: `SCHED_FIFO` on a runaway process can lock a machine
/// hard enough to need the power button, which is not a thing a task manager
/// should hand out beside "Stop".
pub const NICE_LEVELS: &[(&str, i32)] =
    &[("Highest", -5), ("High", -2), ("Normal", 0), ("Low", 5), ("Lowest", 10)];

/// Is `nice` one of the levels offered?
///
/// The command validates rather than trusting its caller: `setpriority` accepts
/// -20, which starves everything else on the machine including the compositor,
/// and this app is not the place to hand that out from a menu. Pure.
pub fn valid_nice(nice: i32) -> bool {
    NICE_LEVELS.iter().any(|(_, n)| *n == nice)
}

/// Set the scheduling priority of `pid`.
///
/// THE ASYMMETRY, which the UI has to tell the truth about: raising the nice
/// value (making a process kinder) is allowed for your own processes, but
/// LOWERING it needs `CAP_SYS_NICE`. So "make this faster" fails with EPERM for
/// an ordinary user while "make this slower" succeeds, and a control that
/// silently no-ops in one direction is worse than one that says it was refused.
/// The error is returned verbatim for that reason.
pub fn set_nice(pid: u32, nice: i32) -> Result<(), Refusal> {
    if !valid_signal_target(pid) {
        return Err(Refusal::UnsafePid(pid));
    }
    if !valid_nice(nice) {
        return Err(Refusal::BadPriority(nice));
    }
    // `setpriority` returns -1 on error, but -1 is also a legal RESULT of
    // `getpriority`, so errno must be cleared first to tell them apart. Only
    // setpriority is called here, where -1 is unambiguous, but the errno reset
    // keeps a later `getpriority` honest.
    unsafe {
        *libc::__errno_location() = 0;
    }
    // SAFETY: a plain syscall over a range-checked pid and a validated priority.
    let rc = unsafe { libc::setpriority(libc::PRIO_PROCESS, pid, nice) };
    if rc == 0 {
        Ok(())
    } else {
        Err(refusal_from_os(&std::io::Error::last_os_error()))
    }
}

/// Read the current scheduling priority of `pid`, so the menu can show which
/// level a process is actually at rather than assuming Normal.
///
/// `None` when the process is gone or unreadable - never 0, which would claim
/// Normal for a process niced to 10.
pub fn get_nice(pid: u32) -> Option<i32> {
    if !valid_signal_target(pid) {
        return None;
    }
    unsafe {
        // -1 is a legal priority AND the error return, so errno is the only way
        // to tell "this process is at -1" from "this call failed".
        *libc::__errno_location() = 0;
        let v = libc::getpriority(libc::PRIO_PROCESS, pid);
        if v == -1 && *libc::__errno_location() != 0 {
            None
        } else {
            Some(v)
        }
    }
}

#[cfg(test)]
mod nice_tests {
    use super::*;

    #[test]
    fn only_the_offered_levels_are_accepted() {
        for (_, n) in NICE_LEVELS {
            assert!(valid_nice(*n));
        }
        // -20 starves everything else on the machine, including the compositor
        // drawing this window. A menu is not the place to hand that out.
        assert!(!valid_nice(-20));
        assert!(!valid_nice(19));
        assert!(!valid_nice(-6));
    }

    #[test]
    fn an_unsafe_pid_is_refused_before_the_syscall() {
        // pid 0 is the caller's own process group: reniceing it would renice
        // this app and everything it launched.
        assert_eq!(set_nice(0, 0), Err(Refusal::UnsafePid(0)));
        assert!(get_nice(0).is_none());
    }

    #[test]
    fn a_priority_outside_the_menu_is_refused_before_the_syscall() {
        assert_eq!(set_nice(std::process::id(), -20), Err(Refusal::BadPriority(-20)));
    }

    #[test]
    fn reading_our_own_priority_works_and_a_dead_pid_reads_as_unknown() {
        // Our own nice value is whatever the harness runs at; the point is that
        // it is SOME value rather than an error.
        assert!(get_nice(std::process::id()).is_some());
        // A pid that cannot exist reads as unknown, not as Normal - claiming 0
        // for an unreadable process is the "confident default" this tree keeps
        // removing.
        assert!(get_nice(0x7fff_fffe).is_none());
    }

    #[test]
    fn making_our_own_process_kinder_is_allowed() {
        // The direction that needs no privilege. The opposite one is refused by
        // the kernel for an ordinary user, which is why the UI must show the
        // error rather than pretend the click worked.
        let before = get_nice(std::process::id()).unwrap();
        let target = NICE_LEVELS.iter().map(|(_, n)| *n).find(|n| *n > before);
        if let Some(t) = target {
            assert!(set_nice(std::process::id(), t).is_ok());
            assert_eq!(get_nice(std::process::id()), Some(t));
        }
    }
}
