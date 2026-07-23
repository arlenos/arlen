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

/// Parse `--landlock-exec` arguments: the writable dirs, then `--`, then the
/// program and its argv. Pure, so the split is unit-tested without applying
/// Landlock or exec'ing. Returns `(writable_dirs, program_argv)` where
/// `program_argv[0]` is the program.
pub fn parse_landlock_exec(args: &[String]) -> Result<(Vec<PathBuf>, Vec<String>), LandlockExecError> {
    let sep = args
        .iter()
        .position(|a| a == "--")
        .ok_or(LandlockExecError::NoSeparator)?;
    let writable: Vec<PathBuf> = args[..sep].iter().map(PathBuf::from).collect();
    let program_argv: Vec<String> = args[sep + 1..].to_vec();
    if program_argv.is_empty() {
        return Err(LandlockExecError::NoProgram);
    }
    Ok((writable, program_argv))
}

/// Run the in-sandbox Landlock wrapper: parse the args, install the Landlock fence
/// over `writable`, then `exec` the program (replacing this process, so the app
/// runs under the fence). Only ever returns an `Err` - a successful `exec` never
/// returns. Fail-closed: a Landlock install failure returns before the exec, so the
/// app is never run unconfined.
#[cfg(target_os = "linux")]
pub fn landlock_exec(args: &[String]) -> Result<std::convert::Infallible, LandlockExecError> {
    use std::os::unix::process::CommandExt;

    let (writable, program_argv) = parse_landlock_exec(args)?;
    crate::landlock_apply::apply_landlock(&writable).map_err(LandlockExecError::Landlock)?;

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
        let (writable, prog) = parse_landlock_exec(&args).unwrap();
        assert_eq!(writable, vec![PathBuf::from("/a"), PathBuf::from("/b/c")]);
        assert_eq!(prog, vec!["/usr/bin/echo".to_string(), "hi".to_string()]);
    }

    #[test]
    fn no_writable_dirs_is_allowed_read_only() {
        // An empty writable set is valid: the app gets a read-only fence (the most
        // restrictive), same as `apply_landlock(&[])`.
        let args: Vec<String> = ["--", "/usr/bin/true"].iter().map(|s| s.to_string()).collect();
        let (writable, prog) = parse_landlock_exec(&args).unwrap();
        assert!(writable.is_empty());
        assert_eq!(prog, vec!["/usr/bin/true".to_string()]);
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
}
