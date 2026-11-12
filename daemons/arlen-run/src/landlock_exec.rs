//! The in-sandbox Landlock wrapper mode.
//!
//! Landlock confines the process it is installed on and every thread/child that
//! process later spawns; it can only ever stack TIGHTER. Applying it in the
//! launcher's child before `execve("bwrap")` therefore confines bwrap's OWN setup
//! (its user-namespace + newroot writes), which breaks bwrap before the app ever
//! runs (see `spawn::child_pre_exec`). The correct place for the filesystem
//! defense-in-depth layer is INSIDE the sandbox, applied to the app after bwrap has
//! built its mount namespace: bwrap execs `arlen-run --landlock-exec <writable>...
//! -- <program> <args>...` as the app's stand-in, this mode applies Landlock and
//! then `exec`s the real program, so the Landlock domain the app inherits grants
//! read over the pruned bwrap view and write ONLY under the app's own dirs -
//! independent of bwrap's mount confinement, a genuine second layer.
//!
//! This module is the mechanism (the mode + its argument parse); wiring bwrap to
//! invoke it (binding arlen-run into the sandbox and re-pointing `bwrap_argv`) is a
//! separate, adversarially-reviewed slice.

use std::path::PathBuf;

/// The standard writable pseudo-files/dirs a normal app needs even under a
/// read-only-root fence: the throwaway character devices and the sandbox's own
/// tmpfs. Under Landlock a read-only root denies opening these `O_WRONLY`, which
/// breaks ordinary apps (`2>/dev/null` redirects, toolkit shared memory,
/// tempfiles), so the app fence grants write on them IN ADDITION to the app's own
/// dirs. They are safe to grant: the devices are throwaway, and `/tmp` + `/dev/shm`
/// are the sandbox's private tmpfs (bwrap's own mounts), not the app's data or the
/// host filesystem. An entry absent from the sandbox is skipped fail-safe by
/// [`crate::landlock_apply::apply_landlock`], so listing all of them is harmless.
const STANDARD_WRITABLE: &[&str] = &[
    "/dev/null",
    "/dev/zero",
    "/dev/full",
    "/dev/random",
    "/dev/urandom",
    "/dev/tty",
    "/dev/shm",
    "/tmp",
];

/// A failure of the `--landlock-exec` wrapper mode. Each maps to a fail-closed exit
/// code so the app never runs unconfined or with a half-installed fence.
#[derive(Debug)]
pub enum LandlockExecError {
    /// No `--` separating the writable dirs from the program.
    NoSeparator,
    /// Nothing after `--` to exec.
    NoProgram,
    /// The Landlock ruleset could not be installed (or the kernel did not enforce
    /// it). Fail-closed: refuse to exec the app rather than run it unconfined.
    Landlock(std::io::Error),
    /// `exec` of the program failed (e.g. not found). The app did not start.
    Exec(std::io::Error),
}

impl std::fmt::Display for LandlockExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LandlockExecError::NoSeparator => write!(f, "missing `--` before the program"),
            LandlockExecError::NoProgram => write!(f, "no program after `--`"),
            LandlockExecError::Landlock(e) => write!(f, "landlock: {e}"),
            LandlockExecError::Exec(e) => write!(f, "exec: {e}"),
        }
    }
}

/// The flag that ends the writable dirs and starts the sockets the app may dial.
/// No writable dir can be mistaken for it: those are absolute paths.
const DIAL_FLAG: &str = "--dial";

/// What `--landlock-exec` was asked to do.
#[derive(Debug, PartialEq, Eq)]
pub struct LandlockExecArgs {
    /// The dirs the app may write under.
    pub writable: Vec<PathBuf>,
    /// The path sockets the app may connect to. `None` when `--dial` was not
    /// given, which leaves connecting unfenced; `Some` of nothing fences it and
    /// opens no socket.
    pub dial: Option<Vec<PathBuf>>,
    /// The program and its argv.
    pub program: Vec<String>,
}

/// Parse `--landlock-exec` arguments: the writable dirs, optionally `--dial` and
/// the sockets, then `--`, then the program and its argv. Pure, so the split is
/// unit-tested without applying Landlock or exec'ing.
pub fn parse_landlock_exec(args: &[String]) -> Result<LandlockExecArgs, LandlockExecError> {
    let sep = args
        .iter()
        .position(|a| a == "--")
        .ok_or(LandlockExecError::NoSeparator)?;
    let head = &args[..sep];
    let (writable, dial) = match head.iter().position(|a| a == DIAL_FLAG) {
        Some(at) => (&head[..at], Some(head[at + 1..].iter().map(PathBuf::from).collect())),
        None => (head, None),
    };
    let program: Vec<String> = args[sep + 1..].to_vec();
    if program.is_empty() {
        return Err(LandlockExecError::NoProgram);
    }
    Ok(LandlockExecArgs {
        writable: writable.iter().map(PathBuf::from).collect(),
        dial,
        program,
    })
}

/// Build the program argv that has bwrap invoke the in-sandbox fence: the app is
/// run as `<arlen_run> --landlock-exec <writable>... [--dial <socket>...] --
/// <program>...`, so bwrap execs arlen-run - which the caller must bind read-only
/// into the sandbox at the `arlen_run` path - and arlen-run fences the writable
/// set and the sockets and execs the real program. The inverse of
/// [`parse_landlock_exec`]. Pure.
pub fn landlock_exec_program(
    arlen_run: &str,
    writable: &[PathBuf],
    dial: Option<&[PathBuf]>,
    program: &[String],
) -> Vec<String> {
    let mut argv = vec![arlen_run.to_string(), "--landlock-exec".to_string()];
    argv.extend(writable.iter().map(|p| p.to_string_lossy().into_owned()));
    if let Some(dial) = dial {
        argv.push(DIAL_FLAG.to_string());
        argv.extend(dial.iter().map(|p| p.to_string_lossy().into_owned()));
    }
    argv.push("--".to_string());
    argv.extend(program.iter().cloned());
    argv
}

/// Run the in-sandbox Landlock wrapper: parse the args, install the Landlock fence
/// over `writable`, then `exec` the program (replacing this process, so the app
/// runs under the fence). Only ever returns an `Err` - a successful `exec` never
/// returns. Fail-closed: a Landlock install failure returns before the exec, so the
/// app is never run unconfined.
#[cfg(target_os = "linux")]
pub fn landlock_exec(args: &[String]) -> Result<std::convert::Infallible, LandlockExecError> {
    use std::os::unix::process::CommandExt;

    let LandlockExecArgs { mut writable, dial, program: program_argv } = parse_landlock_exec(args)?;
    // Grant the standard writable pseudo-files/dirs alongside the app's own dirs so
    // the read-only-root fence does not deny writes every app needs (`/dev/null`,
    // the sandbox tmpfs). Narrow + safe (throwaway devices + the sandbox's private
    // tmpfs), never a widening of the app's real data grant.
    writable.extend(STANDARD_WRITABLE.iter().map(PathBuf::from));
    crate::landlock_apply::apply_landlock(&writable, dial.as_deref()).map_err(LandlockExecError::Landlock)?;

    // Replace this process with the app, now inside the Landlock domain. `exec`
    // returns only on failure.
    let err = std::process::Command::new(&program_argv[0])
        .args(&program_argv[1..])
        .exec();
    Err(LandlockExecError::Exec(err))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_writable_dirs_then_the_program() {
        let args: Vec<String> = ["/a", "/b/c", "--", "/usr/bin/echo", "hi"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let parsed = parse_landlock_exec(&args).unwrap();
        assert_eq!(parsed.writable, vec![PathBuf::from("/a"), PathBuf::from("/b/c")]);
        assert_eq!(parsed.dial, None);
        assert_eq!(parsed.program, vec!["/usr/bin/echo".to_string(), "hi".to_string()]);
    }

    #[test]
    fn no_writable_dirs_is_allowed_read_only() {
        // An empty writable set is valid: the app gets a read-only fence (the most
        // restrictive), same as `apply_landlock(&[])`.
        let args: Vec<String> = ["--", "/usr/bin/true"].iter().map(|s| s.to_string()).collect();
        let parsed = parse_landlock_exec(&args).unwrap();
        assert!(parsed.writable.is_empty());
        assert_eq!(parsed.program, vec!["/usr/bin/true".to_string()]);
    }

    #[test]
    fn a_missing_separator_is_rejected() {
        let args: Vec<String> = ["/a", "/usr/bin/echo"].iter().map(|s| s.to_string()).collect();
        assert!(matches!(
            parse_landlock_exec(&args),
            Err(LandlockExecError::NoSeparator)
        ));
    }

    #[test]
    fn nothing_after_the_separator_is_rejected() {
        let args: Vec<String> = ["/a", "--"].iter().map(|s| s.to_string()).collect();
        assert!(matches!(
            parse_landlock_exec(&args),
            Err(LandlockExecError::NoProgram)
        ));
    }

    #[test]
    fn the_wrap_and_parse_round_trip() {
        // What bwrap is handed (`landlock_exec_program`) is exactly what the mode
        // parses back (`parse_landlock_exec`), so the writable set and program the
        // fence applies match what the launcher intended.
        let writable = vec![PathBuf::from("/home/u/.local/share/arlen/apps/a"), PathBuf::from("/x")];
        let program = vec!["/app/bin".to_string(), "--flag".to_string(), "v".to_string()];
        let dial = vec![PathBuf::from("/run/arlen/knowledge.sock")];
        let argv =
            landlock_exec_program("/usr/lib/arlen/arlen-run", &writable, Some(&dial), &program);
        assert_eq!(argv[0], "/usr/lib/arlen/arlen-run");
        assert_eq!(argv[1], "--landlock-exec");
        // The mode re-parses the SAME writable set and program from the tail.
        let parsed = parse_landlock_exec(&argv[2..]).unwrap();
        assert_eq!(parsed.writable, writable);
        assert_eq!(parsed.dial, Some(dial));
        assert_eq!(parsed.program, program);
    }

    #[test]
    fn a_dial_flag_with_no_socket_still_fences_connecting() {
        // "No socket" and "not asked" must stay apart: the first opens nothing,
        // the second leaves connecting alone.
        let args: Vec<String> =
            ["/a", "--dial", "--", "/usr/bin/true"].iter().map(|s| s.to_string()).collect();
        let parsed = parse_landlock_exec(&args).unwrap();
        assert_eq!(parsed.writable, vec![PathBuf::from("/a")]);
        assert_eq!(parsed.dial, Some(Vec::new()));
    }

    #[test]
    fn a_dial_flag_in_the_program_is_the_programs_own() {
        let args: Vec<String> =
            ["/a", "--", "/usr/bin/tool", "--dial", "x"].iter().map(|s| s.to_string()).collect();
        let parsed = parse_landlock_exec(&args).unwrap();
        assert_eq!(parsed.dial, None);
        assert_eq!(parsed.program.len(), 3);
    }
}
