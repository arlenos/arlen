//! System info for the About settings page.
//!
//! Read-only stats: kernel version, compositor build, daemon statuses. Nothing
//! here makes a token-authenticated round-trip; each daemon is checked by the
//! cheapest signal that is actually true of it.
//!
//! For three of them that is socket existence, the same pattern as
//! `commands/knowledge.rs::knowledge_stats_get`. For the install daemon it is bus
//! name ownership, because that daemon binds no socket - probing a path for it
//! reported "down" on every system ever booted, which is where the note on
//! `installd_name_has_owner` comes from.

use std::path::Path;
use std::process::Command;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    /// Arlen release tag, read from
    /// `/usr/share/arlen/version` (written by installd at install
    /// time). `null` on dev systems where the file isn't present.
    pub arlen_version: Option<String>,
    /// `uname -r` output. `null` on systems without uname.
    pub kernel: Option<String>,
    /// `WAYLAND_DISPLAY` env var.
    pub wayland_display: Option<String>,
    pub daemons: Vec<DaemonStatus>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonStatus {
    pub name: String,
    pub running: bool,
    /// What was checked to decide `running`. A socket path for the daemons that
    /// bind one, and a bus name for the install daemon, which does not. Surfaced
    /// for debug - the UI shows it on hover, so it has to name the thing that was
    /// actually looked at rather than a path that sounds plausible.
    pub probe_path: String,
}

#[tauri::command]
pub async fn about_get_system_info() -> SystemInfo {
    SystemInfo {
        arlen_version: read_version_file(),
        kernel: kernel_release(),
        wayland_display: std::env::var("WAYLAND_DISPLAY").ok(),
        daemons: daemon_statuses().await,
    }
}

fn read_version_file() -> Option<String> {
    std::fs::read_to_string("/usr/share/arlen/version")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn kernel_release() -> Option<String> {
    let output = Command::new("uname").arg("-r").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8(output.stdout).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

async fn daemon_statuses() -> Vec<DaemonStatus> {
    vec![
        DaemonStatus {
            name: "Knowledge Graph".into(),
            running: knowledge_socket_exists(),
            probe_path: knowledge_socket_path_string(),
        },
        DaemonStatus {
            name: "Notification".into(),
            running: notification_socket_exists(),
            probe_path: notification_socket_path_string(),
        },
        DaemonStatus {
            name: "Event Bus".into(),
            running: event_bus_socket_exists(),
            probe_path: "/run/arlen/event-bus-consumer.sock".into(),
        },
        DaemonStatus {
            name: "Install Daemon".into(),
            running: installd_name_has_owner().await,
            probe_path: INSTALLD_BUS_NAME.into(),
        },
    ]
}

// ── Socket-existence probes ────────────────────────────────────────

fn knowledge_socket_path_string() -> String {
    // Both env names, via the SDK. This read only `ARLEN_DAEMON_SOCKET` and then
    // fell through to XDG, so on a booted image - where the launcher exports
    // `ARLEN_KNOWLEDGE_SOCKET` - the probe checked a path nothing binds and this
    // page reported the knowledge daemon as not running while it was running.
    os_sdk::runtime::knowledge_socket_path()
        .to_string_lossy()
        .into_owned()
}

fn knowledge_socket_exists() -> bool {
    Path::new(&knowledge_socket_path_string()).exists()
}

fn notification_socket_path_string() -> String {
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        return format!("{xdg}/arlen/notification.sock");
    }
    // No reasonable system-bus fallback for the notification
    // daemon — it's per-user.
    String::new()
}

fn notification_socket_exists() -> bool {
    let p = notification_socket_path_string();
    !p.is_empty() && Path::new(&p).exists()
}

fn event_bus_socket_exists() -> bool {
    Path::new("/run/arlen/event-bus-consumer.sock").exists()
}

/// The install daemon is reached by bus name, not by socket.
const INSTALLD_BUS_NAME: &str = "org.arlen.InstallDaemon1";

/// Whether anything owns the install daemon's bus name.
///
/// THE SOCKET THIS USED TO PROBE HAS NEVER EXISTED. It looked for
/// `$XDG_RUNTIME_DIR/arlen/installd.sock`, and `installd.sock` appears exactly
/// once in the whole tree - here, in the code looking for it. `daemons/installd`
/// binds no Unix listener at all; it owns `org.arlen.InstallDaemon1` on the
/// session bus and that is its entire interface. So this row reported the install
/// daemon as down unconditionally, on every system, including one where it was
/// running perfectly.
///
/// The knowledge row above carries a comment about the same defect found the same
/// way, with one difference that matters: that probe named a real socket at the
/// wrong path, so a path fix could work. This one had no right answer available -
/// nothing this function could have looked for on disk would ever have been true.
///
/// `NameHasOwner` is the question actually being asked. It is false when nothing
/// provides the daemon, which is the state of the current image, and true the
/// moment something does.
async fn installd_name_has_owner() -> bool {
    let Ok(conn) = zbus::Connection::session().await else {
        return false;
    };
    let Ok(dbus) = zbus::fdo::DBusProxy::new(&conn).await else {
        return false;
    };
    match zbus::names::BusName::try_from(INSTALLD_BUS_NAME) {
        Ok(name) => dbus.name_has_owner(name).await.unwrap_or(false),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `read_version_file` returns `None` cleanly when the file is
    /// missing — that's the dev-system default and must not crash.
    #[test]
    fn version_file_missing_is_none() {
        // Test runs without /usr/share/arlen/version on most CI;
        // we don't assert specific value, only that the call returns
        // a well-formed Option (no panics).
        let _ = read_version_file();
    }

    /// Daemon-status list is exhaustive — all four daemons present
    /// regardless of host state. Catches accidental list-truncation.
    #[tokio::test]
    async fn daemon_statuses_lists_all_four() {
        let list = daemon_statuses().await;
        assert_eq!(list.len(), 4, "expected 4 daemons, got {}", list.len());
        let names: Vec<&str> = list.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"Knowledge Graph"));
        assert!(names.contains(&"Notification"));
        assert!(names.contains(&"Event Bus"));
        assert!(names.contains(&"Install Daemon"));

        // The install daemon binds no socket, so its probe has to name the bus.
        // A path here would be the defect this replaced: a plausible-looking
        // string nothing ever creates, and a row that reads "down" forever.
        let installd = list.iter().find(|d| d.name == "Install Daemon").unwrap();
        assert_eq!(installd.probe_path, "org.arlen.InstallDaemon1");
        assert!(
            !installd.probe_path.contains(".sock"),
            "nothing in the tree ever binds an installd socket"
        );
    }

    /// `about_get_system_info` always returns — fields may be null
    /// but the call itself must succeed on any host.
    #[tokio::test]
    async fn system_info_returns_well_formed_struct() {
        let info = about_get_system_info().await;
        assert_eq!(info.daemons.len(), 4);
    }

    #[test]
    fn system_info_serialises_as_camel_case() {
        let info = SystemInfo {
            arlen_version: Some("0.1.0".into()),
            kernel: Some("6.0.0".into()),
            wayland_display: Some("wayland-1".into()),
            daemons: vec![],
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("arlenVersion"));
        assert!(json.contains("waylandDisplay"));
        assert!(!json.contains("arlen_version"));
    }
}
