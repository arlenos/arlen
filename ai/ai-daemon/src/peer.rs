//! Peer-credential resolution for ai-daemon callers.
//!
//! The in-flight rate limit must be keyed on a *stable* caller
//! identity, not the per-connection D-Bus unique name. A unique name
//! (`:1.42`) is per-connection, so a caller could otherwise open
//! extra connections to multiply its quota. This module resolves the
//! caller's PID via `GetConnectionUnixProcessID` and reads
//! `/proc/{pid}/exe` to obtain the canonical executable path, which
//! serves as that stable key.
//!
//! Unlike the ai-proxy peer check, this module does not gate on an
//! executable allowlist: any app may submit AI queries. It needs the
//! executable path only for rate-limit accounting. The unique name is
//! still carried alongside because result retrieval is authorised
//! connection-precisely (a sibling connection of the same app must
//! not poll another connection's query).

use crate::service::CallerIdentity;

/// Errors raised while resolving peer credentials.
#[derive(Debug, thiserror::Error)]
pub enum PeerError {
    /// D-Bus message carried no sender.
    #[error("message has no sender")]
    NoSender,
    /// `GetConnectionUnixProcessID` failed.
    #[error("could not resolve caller PID: {0}")]
    PidLookup(String),
    /// `/proc/{pid}/exe` could not be read.
    #[error("could not read /proc/{pid}/exe: {error}")]
    ExeLookup {
        /// Caller PID.
        pid: u32,
        /// Reason for failure.
        error: String,
    },
}

/// Resolve the caller identity from a D-Bus message header.
///
/// The PID is mandatory (no sender / no PID is a hard error). The
/// executable path is best-effort: when `/proc/{pid}/exe` cannot be
/// read - a sandboxed, non-dumpable same-uid caller, which much systemd
/// hardening produces - the stable key falls back to `pid:{pid}`. That
/// is still stable and still defeats the connection-multiplication
/// attack (one process keeps one PID across any number of bus
/// connections), unlike the per-connection unique name, so the query is
/// not refused over an accounting detail. Any app may submit AI queries,
/// including confined ones, so resolution must not require the exe.
pub async fn resolve(
    header: &zbus::message::Header<'_>,
    connection: &zbus::Connection,
) -> Result<CallerIdentity, PeerError> {
    let sender = header.sender().ok_or(PeerError::NoSender)?;
    let unique_bus_name = sender.to_string();

    let dbus_proxy = zbus::fdo::DBusProxy::new(connection)
        .await
        .map_err(|e| PeerError::PidLookup(format!("DBusProxy: {e}")))?;
    let bus_name = zbus::names::BusName::try_from(unique_bus_name.as_str())
        .map_err(|e| PeerError::PidLookup(format!("parse bus name: {e}")))?;
    let pid = dbus_proxy
        .get_connection_unix_process_id(bus_name)
        .await
        .map_err(|e| PeerError::PidLookup(e.to_string()))?;
    // Best-effort: a non-dumpable caller's /proc/{pid}/exe is unreadable by a
    // same-uid peer, so fall back to the PID (kernel-attested, one per process)
    // rather than refuse the query. The PID resists the quota-multiplication
    // attack the exe key guards against, unlike the per-connection unique name.
    let stable_id = match std::fs::read_link(format!("/proc/{pid}/exe")) {
        Ok(exe_path) => exe_path.to_string_lossy().to_string(),
        Err(_) => format!("pid:{pid}"),
    };

    Ok(CallerIdentity {
        unique_bus_name,
        stable_id,
    })
}
