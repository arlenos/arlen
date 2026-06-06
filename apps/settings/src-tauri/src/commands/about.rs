//! System info for the About settings page.
//!
//! Read-only stats: kernel version, compositor build, daemon
//! statuses. Uses the same socket-existence pattern as
//! `commands/knowledge.rs::knowledge_stats_get` — file-based
//! signals, no token-authenticated daemon round-trip.

use std::path::Path;
use std::process::Command;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    /// Lunaris release tag, read from
    /// `/usr/share/lunaris/version` (written by installd at install
    /// time). `null` on dev systems where the file isn't present.
    pub lunaris_version: Option<String>,
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
    /// Path of the socket / file used to test liveness. Surfaced
    /// for debug — UI shows it on hover.
    pub probe_path: String,
}

#[tauri::command]
pub fn about_get_system_info() -> SystemInfo {
    SystemInfo {
        lunaris_version: read_version_file(),
        kernel: kernel_release(),
        wayland_display: std::env::var("WAYLAND_DISPLAY").ok(),
        daemons: daemon_statuses(),
    }
}

fn read_version_file() -> Option<String> {
    std::fs::read_to_string("/usr/share/lunaris/version")
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

fn daemon_statuses() -> Vec<DaemonStatus> {
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
            probe_path: "/run/lunaris/event-bus-consumer.sock".into(),
        },
        DaemonStatus {
            name: "Install Daemon".into(),
            running: installd_socket_exists(),
            probe_path: installd_socket_path_string(),
        },
    ]
}

// ── Socket-existence probes ────────────────────────────────────────

fn knowledge_socket_path_string() -> String {
    if let Ok(p) = std::env::var("LUNARIS_DAEMON_SOCKET") {
        return p;
    }
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        return format!("{xdg}/lunaris/knowledge.sock");
    }
    "/run/lunaris/knowledge.sock".into()
}

fn knowledge_socket_exists() -> bool {
    Path::new(&knowledge_socket_path_string()).exists()
}

fn notification_socket_path_string() -> String {
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        return format!("{xdg}/lunaris/notification.sock");
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
    Path::new("/run/lunaris/event-bus-consumer.sock").exists()
}

fn installd_socket_path_string() -> String {
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        return format!("{xdg}/lunaris/installd.sock");
    }
    "/run/lunaris/installd.sock".into()
}

fn installd_socket_exists() -> bool {
    Path::new(&installd_socket_path_string()).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `read_version_file` returns `None` cleanly when the file is
    /// missing — that's the dev-system default and must not crash.
    #[test]
    fn version_file_missing_is_none() {
        // Test runs without /usr/share/lunaris/version on most CI;
        // we don't assert specific value, only that the call returns
        // a well-formed Option (no panics).
        let _ = read_version_file();
    }

    /// Daemon-status list is exhaustive — all four daemons present
    /// regardless of host state. Catches accidental list-truncation.
    #[test]
    fn daemon_statuses_lists_all_four() {
        let list = daemon_statuses();
        assert_eq!(list.len(), 4, "expected 4 daemons, got {}", list.len());
        let names: Vec<&str> = list.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"Knowledge Graph"));
        assert!(names.contains(&"Notification"));
        assert!(names.contains(&"Event Bus"));
        assert!(names.contains(&"Install Daemon"));
    }

    /// `about_get_system_info` always returns — fields may be null
    /// but the call itself must succeed on any host.
    #[test]
    fn system_info_returns_well_formed_struct() {
        let info = about_get_system_info();
        assert_eq!(info.daemons.len(), 4);
    }

    #[test]
    fn system_info_serialises_as_camel_case() {
        let info = SystemInfo {
            lunaris_version: Some("0.1.0".into()),
            kernel: Some("6.0.0".into()),
            wayland_display: Some("wayland-1".into()),
            daemons: vec![],
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("lunarisVersion"));
        assert!(json.contains("waylandDisplay"));
        assert!(!json.contains("lunaris_version"));
    }
}
