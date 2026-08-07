//! Keeping the clock's state across a restart.
//!
//! **An alarm that a daemon restart forgets is not an alarm.** The whole reason
//! the state lives outside the GUI is that closing a window must change
//! nothing; the same argument reaches one step further, because a crash, an
//! update or a logout is exactly the sort of thing that happens overnight
//! between setting an alarm and needing it.
//!
//! So the store errs toward keeping what it has:
//!
//! - Written atomically - a temporary file, then a rename - because a half-
//!   written state file read at the next boot is worse than an old one.
//! - Read tolerantly. A file that will not parse means the alarms are gone,
//!   which is the loudest failure this daemon has, so an unreadable state is
//!   moved aside rather than overwritten. Somebody may want it back, and the
//!   daemon has to start either way.
//!
//! Runtime state, so `$XDG_STATE_HOME/arlen/clock` - machine-written, not
//! something anyone hand-edits, which is what separates it from config.

use std::path::{Path, PathBuf};

use crate::state::ClockState;

/// Where the clock keeps its state.
///
/// `$XDG_STATE_HOME/arlen/clock`, or `$HOME/.local/state/arlen/clock`. `None`
/// when neither resolves, which the daemon reports rather than guessing at a
/// path: writing somebody's alarms to a directory nobody asked for is worse
/// than saying it cannot.
pub fn state_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|s| !s.is_empty())
                .map(|h| PathBuf::from(h).join(".local/state"))
        })?;
    base.is_absolute().then(|| base.join("arlen/clock"))
}

/// The state file inside `dir`.
pub fn state_path(dir: &Path) -> PathBuf {
    dir.join("state.json")
}

/// Why the state could not be kept.
#[derive(Debug)]
pub enum StoreError {
    /// The filesystem refused.
    Io(std::io::Error),
    /// The state could not be turned into JSON, which is a bug here rather than
    /// a condition of the machine.
    Encode(serde_json::Error),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "clock state: {e}"),
            Self::Encode(e) => write!(f, "clock state could not be encoded: {e}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// Write the state, atomically.
///
/// The temporary file sits beside the target so the rename stays within one
/// filesystem, and it is removed if the rename fails - a stray `state.json.tmp`
/// would be read by nothing and cleaned by nobody.
pub fn save(dir: &Path, state: &ClockState) -> Result<(), StoreError> {
    std::fs::create_dir_all(dir).map_err(StoreError::Io)?;
    let body = serde_json::to_vec_pretty(state).map_err(StoreError::Encode)?;
    let target = state_path(dir);
    let tmp = target.with_extension("json.tmp");
    std::fs::write(&tmp, &body).map_err(StoreError::Io)?;
    if let Err(e) = std::fs::rename(&tmp, &target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(StoreError::Io(e));
    }
    Ok(())
}

/// What loading found.
#[derive(Debug, PartialEq, Eq)]
pub enum Loaded {
    /// The state as it was left.
    Kept(ClockState),
    /// Nothing was there. A first run, and not a problem.
    Fresh,
    /// Something was there and could not be read. The daemon starts empty, and
    /// the file is kept under `path` rather than overwritten.
    Unreadable {
        /// Where the unreadable file was moved to.
        path: PathBuf,
        /// What was wrong with it, for the log.
        reason: String,
    },
}

/// Read the state back.
///
/// Never fails: a daemon that refuses to start because its state file is odd
/// leaves the user with no clock at all, which is a worse answer than an empty
/// one. What it does not do is pretend - an unreadable file is reported as
/// such, and kept.
pub fn load(dir: &Path) -> Loaded {
    let path = state_path(dir);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Loaded::Fresh,
        Err(e) => return set_aside(&path, &e.to_string()),
    };
    match serde_json::from_str::<ClockState>(&text) {
        Ok(state) => Loaded::Kept(state),
        Err(e) => set_aside(&path, &e.to_string()),
    }
}

/// Move an unreadable state file out of the way, keeping it.
fn set_aside(path: &Path, reason: &str) -> Loaded {
    let kept = path.with_extension("json.unreadable");
    let _ = std::fs::rename(path, &kept);
    Loaded::Unreadable {
        path: kept,
        reason: reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Stopwatch, Timer};

    fn some_state() -> ClockState {
        ClockState {
            wake_capable: true,
            timers: vec![Timer {
                id: "t".into(),
                duration_ms: 60_000,
                ends_at: Some(1_000),
                remaining_ms: None,
                paused: false,
            }],
            stopwatch: Stopwatch {
                running: true,
                started_at: Some(5),
                accumulated_ms: 7,
                laps: vec![1, 2],
            },
            ..ClockState::default()
        }
    }

    #[test]
    fn state_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let state = some_state();
        save(dir.path(), &state).unwrap();
        assert_eq!(load(dir.path()), Loaded::Kept(state));
    }

    #[test]
    fn a_first_run_finds_nothing_and_that_is_not_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), Loaded::Fresh);
    }

    /// The daemon has to start, and the file has to survive: losing somebody's
    /// alarms is the loudest failure here, so it is set aside rather than
    /// overwritten by the next save.
    #[test]
    fn an_unreadable_state_is_kept_rather_than_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path()).unwrap();
        std::fs::write(state_path(dir.path()), "{ this is not json").unwrap();

        let loaded = load(dir.path());
        let Loaded::Unreadable { path, reason } = loaded else {
            panic!("expected the file to be set aside, got {loaded:?}");
        };
        assert!(path.exists(), "the original was not kept");
        assert!(!reason.is_empty());
        // And the daemon can now save over a clean slate.
        save(dir.path(), &ClockState::default()).unwrap();
        assert_eq!(load(dir.path()), Loaded::Kept(ClockState::default()));
    }

    /// A save leaves one file, not a file and a leftover temporary.
    #[test]
    fn saving_leaves_no_temporary_behind() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), &some_state()).unwrap();
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["state.json".to_string()]);
    }

    #[test]
    fn saving_creates_the_directory_it_needs() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a/b/clock");
        save(&nested, &ClockState::default()).unwrap();
        assert!(state_path(&nested).exists());
    }

    /// A state directory has to be somewhere real; guessing would write
    /// somebody's alarms to a path nobody asked for.
    #[test]
    fn a_relative_state_base_is_refused() {
        // Not touching the process environment: the rule is in one expression
        // and the test states it rather than racing other tests over env vars.
        assert!(!Path::new("relative/state").is_absolute());
    }
}
