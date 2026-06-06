//! Lunaris Anomaly Detector.
//!
//! `lunaris-anomalyd` watches the system audit log (foundation §8.4.8)
//! for behavioural anomalies and raises advisory alerts. It is a
//! standalone system daemon, deliberately separate from both the
//! component it watches and the audit daemon it reads: foundation
//! §8.4 notes that a compromised Graph Daemon dumping the graph "will
//! generate anomaly alerts unless the attacker also compromises the
//! Audit Daemon, which is a separate process" — the detector must
//! observe from outside the process it watches.
//!
//! It is **advisory only**: it dispatches notifications, it never
//! blocks AI activity. Auto-blocking is an opt-in managed-environment
//! policy (foundation §8.4.8), out of scope here.
//!
//! Architecture: `docs/architecture/anomaly-detector.md`.
//!
//! Inputs: the audit read API (`audit-proto::ReadClient`, the reliable
//! by-index poll) and the Event Bus (`audit.*` triggers + `window.*`
//! as a recent-user-activity proxy). Output: `org.freedesktop.
//! Notifications` alerts. The detection heuristics ([`detect`]) are
//! pure functions over injected state and time, so they are tested
//! without sockets or real clocks.

#![deny(unsafe_code)]

use std::path::{Path, PathBuf};

pub mod detect;
pub mod notify;
pub mod source;
pub mod state;

/// The per-user data directory for the detector:
/// `$XDG_DATA_HOME/lunaris/anomaly/`, else
/// `$HOME/.local/share/lunaris/anomaly/`. Errors if no absolute
/// per-user path can be resolved — the detector keeps a small state
/// file there and must not fall back to a world-writable location.
pub fn data_dir() -> std::io::Result<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|s| !s.is_empty())
                .map(|h| PathBuf::from(h).join(".local/share"))
        })
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "cannot resolve a per-user data directory: neither \
                 XDG_DATA_HOME nor HOME is set",
            )
        })?;
    if !base.is_absolute() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("resolved data base {} is not absolute", base.display()),
        ));
    }
    Ok(base.join("lunaris/anomaly"))
}

/// Create `dir` (and parents) and tighten it to mode 0700.
pub fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

/// Current wall-clock time in microseconds since the Unix epoch.
pub fn now_micros() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or(0)
}
