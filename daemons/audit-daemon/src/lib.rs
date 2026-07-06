//! Arlen Audit Daemon.
//!
//! `arlen-auditd` is the sole writer of the system audit log
//! (foundation §8.4.7). Every Knowledge-Graph access, AI action, and
//! permission grant or denial is recorded as one entry in an
//! append-only, hash-chained ledger. Other components never write
//! the log directly: they send audit events over a restricted,
//! peer-authenticated IPC channel and this daemon appends them.
//!
//! Running the audit log as its own process is a deliberate security
//! boundary (§8.4.5): a component compromise — say a Graph Daemon
//! dumping the graph — cannot also suppress or forge the audit trail,
//! so the Anomaly Detector still sees the anomalous activity.
//!
//! Architecture: `docs/architecture/phase-9-gamma-plan.md`.
//!
//! S13.1 scope: the [`ledger`] core — the append-only store, the
//! HMAC hash-chain, and the tamper verifier. The ingest and read
//! sockets are layered on in S13.3 and S13.4.

// The crate is unsafe-free except one audited `libc::getuid` call in
// `ingest` (marked `#[allow(unsafe_code)]` there); `deny` keeps every
// other spot honest.
#![deny(unsafe_code)]

use std::path::{Path, PathBuf};

use tokio::net::UnixListener;

pub mod checkpoint;
pub mod error;
pub mod ingest;
pub mod key;
pub mod ledger;
pub mod read;
pub mod tpm_anchor;

pub use error::{AuditError, Result};

/// The per-user audit data directory: `$XDG_DATA_HOME/arlen/audit/`,
/// else `$HOME/.local/share/arlen/audit/`.
///
/// The audit log is per-user (foundation §8.4.12) and the directory
/// holds the HMAC key, a secret. There is deliberately **no** fallback
/// to a world-writable location such as `/tmp`: another local user
/// could pre-create a directory there and plant a known key. If no
/// absolute per-user data directory can be resolved the daemon fails
/// closed rather than storing key material somewhere unsafe.
pub fn audit_data_dir() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|s| !s.is_empty())
                .map(|h| PathBuf::from(h).join(".local/share"))
        })
        .ok_or_else(|| {
            AuditError::Storage(
                "cannot resolve a per-user audit data directory: \
                 neither XDG_DATA_HOME nor HOME is set"
                    .to_string(),
            )
        })?;
    if !base.is_absolute() {
        return Err(AuditError::Storage(format!(
            "resolved audit data base {} is not an absolute path",
            base.display()
        )));
    }
    Ok(base.join("arlen/audit"))
}

/// Create `dir` (and parents) and tighten it to mode 0700, so only
/// the owning user can traverse into the ledger and the HMAC key.
pub fn ensure_private_dir(dir: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

/// Bind a Unix socket at `path`, mode 0600, after a stale-socket
/// probe: a path still served by a live process is not clobbered —
/// the bind doubles as a singleton guard — while a leftover socket
/// with nothing listening behind it is cleared first.
pub fn bind_unix_socket(path: &Path) -> Result<UnixListener> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if path.exists() {
        match std::os::unix::net::UnixStream::connect(path) {
            Ok(_) => {
                return Err(AuditError::Storage(format!(
                    "{} is already served by a live process",
                    path.display()
                )));
            }
            Err(_) => {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    let listener = UnixListener::bind(path).map_err(|e| {
        AuditError::Storage(format!("bind {}: {e}", path.display()))
    })?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}
