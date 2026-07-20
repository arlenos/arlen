//! Arlen greeter host: the thin Tauri shell for the login screen
//! (`greeter-onboarding-plan.md` §2). The greeter runs before any session
//! and may reach nothing but the greetd socket. These commands are stubs
//! that define the IPC seam the UI renders against; the coder wires them to
//! `greetd_ipc` (profile list, session launch) and the shared
//! `daemons/lock-auth` backend (PAM, the factor tiers, the systemd-homed
//! key release). The Svelte screenshot loop drives a JS mock instead, so the
//! stubs only need to keep the surface honest until the wiring lands: every
//! read fails closed, which the UI renders as "login is not reachable",
//! never as a fake-empty profile list.

use serde::Serialize;
use std::path::{Path, PathBuf};

/// The XDG directories a Wayland session `.desktop` entry may live in, highest
/// precedence first (a local override shadows the system copy of the same id).
/// greetd greeters read these to offer the launchable sessions.
fn wayland_session_dirs() -> Vec<PathBuf> {
    ["/usr/local/share/wayland-sessions", "/usr/share/wayland-sessions"]
        .iter()
        .map(PathBuf::from)
        .collect()
}

/// Extract the `Name=` value from a `.desktop` file, honouring only the
/// `[Desktop Entry]` group and skipping an entry marked `NoDisplay=true`,
/// `Hidden=true`, or lacking an `Exec=` (nothing to launch). Returns `None` when
/// the entry should not be offered. A deliberately small hand parser: the format
/// is a simple INI subset and pulling a full freedesktop crate into the greeter
/// (which may reach nothing but the greetd socket) is not worth it.
fn session_name_from_desktop(contents: &str) -> Option<String> {
    let mut in_entry = false;
    let mut name: Option<String> = None;
    let mut has_exec = false;
    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        match (key.trim(), value.trim()) {
            // Localised names (Name[de]=) are ignored; the unqualified Name wins.
            ("Name", v) if name.is_none() => name = Some(v.to_string()),
            ("Exec", v) if !v.is_empty() => has_exec = true,
            ("NoDisplay", "true") | ("Hidden", "true") => return None,
            _ => {}
        }
    }
    name.filter(|_| has_exec)
}

/// Discover the launchable Wayland sessions across `dirs` (highest precedence
/// first). The id is the `.desktop` basename (the session key greetd launches by);
/// the first directory to define an id wins, so a local session shadows a system
/// one. Non-`.desktop` files, unreadable files and hidden/no-exec entries are
/// skipped. Result is sorted by display name for a stable UI order. Pure over an
/// explicit dir list so it is unit-tested without touching the real system dirs.
fn discover_sessions(dirs: &[PathBuf]) -> Vec<Session> {
    let mut seen: Vec<Session> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|s| s.to_str()).map(str::to_string) else {
                continue;
            };
            if seen.iter().any(|s| s.id == id) {
                continue; // a higher-precedence dir already defined this id
            }
            let Ok(contents) = std::fs::read_to_string(&path) else { continue };
            if let Some(name) = session_name_from_desktop(&contents) {
                seen.push(Session { id, name });
            }
        }
    }
    seen.sort_by(|a, b| a.name.cmp(&b.name));
    seen
}

/// The single fallback session offered when discovery finds nothing installed, so
/// the picker is never empty on a minimal system: the plain Arlen compositor.
fn fallback_session() -> Session {
    Session { id: "arlen".to_string(), name: "Arlen".to_string() }
}

/// One profile offered at login. Mirrors the TypeScript `Profile`.
#[derive(Serialize)]
struct Profile {
    id: String,
    name: String,
    avatar_url: Option<String>,
    kind: String,
    last_used: bool,
    factors: Vec<String>,
}

/// One launchable session. Mirrors the TypeScript `Session`.
#[derive(Serialize)]
struct Session {
    id: String,
    name: String,
}

/// The profiles greetd offers. Stub: fails until wired to greetd.
#[tauri::command]
fn greeter_profiles() -> Result<Vec<Profile>, String> {
    Err("greeter backend not connected".to_string())
}

/// The launchable Wayland sessions, discovered from the XDG
/// `wayland-sessions` directories (the standard greetd greeter source). Always
/// returns at least the plain Arlen fallback so the picker is never empty. This
/// is a pure filesystem read the greeter is allowed even before a session exists;
/// launching the chosen id is greetd's `StartSession`, wired with authenticate.
#[tauri::command]
fn greeter_sessions() -> Result<Vec<Session>, String> {
    let mut sessions = discover_sessions(&wayland_session_dirs());
    if sessions.is_empty() {
        sessions.push(fallback_session());
    }
    Ok(sessions)
}

/// Password authentication for a profile. Stub: the coder routes this to
/// PAM via `daemons/lock-auth`, releasing the systemd-homed key on success.
#[tauri::command]
fn greeter_authenticate(_profile_id: String, _secret: String) -> Result<serde_json::Value, String> {
    Err("greeter backend not connected".to_string())
}

/// Begin a hardware-factor login (FIDO2 / TPM2). Stub: wired to the
/// lock-auth factor abstraction; only the strong factors release the key.
#[tauri::command]
fn greeter_factor_begin(_profile_id: String, _factor: String) -> Result<serde_json::Value, String> {
    Err("greeter backend not connected".to_string())
}

/// Map a frontend power action (the TypeScript `PowerAction`) to its `systemctl`
/// verb. Fail-closed: an unrecognised action returns `None` so nothing runs (the
/// greeter must never turn an unknown string into an arbitrary systemctl call).
fn power_verb(action: &str) -> Option<&'static str> {
    match action {
        "suspend" => Some("suspend"),
        "reboot" => Some("reboot"),
        "power-off" => Some("poweroff"),
        _ => None,
    }
}

/// A power action from the login screen. Runs `systemctl <verb>` (logind grants
/// the greetd greeter these without a session). Only the three known actions map
/// to a verb; anything else is refused rather than passed through.
#[tauri::command]
fn greeter_power(action: String) -> Result<(), String> {
    let verb = power_verb(&action).ok_or_else(|| format!("unknown power action: {action}"))?;
    let status = std::process::Command::new("systemctl")
        .arg(verb)
        .status()
        .map_err(|e| format!("failed to run systemctl {verb}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("systemctl {verb} exited with {status}"))
    }
}

/// Tauri application entry point invoked from `main.rs`.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            greeter_profiles,
            greeter_sessions,
            greeter_authenticate,
            greeter_factor_begin,
            greeter_power
        ])
        .run(tauri::generate_context!())
        .expect("error while running arlen-greeter");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, file: &str, body: &str) {
        std::fs::write(dir.join(file), body).unwrap();
    }

    #[test]
    fn parses_name_and_requires_exec() {
        assert_eq!(
            session_name_from_desktop("[Desktop Entry]\nName=Sway\nExec=/usr/bin/sway\n").as_deref(),
            Some("Sway")
        );
        // No Exec -> nothing to launch -> not offered.
        assert_eq!(session_name_from_desktop("[Desktop Entry]\nName=Broken\n"), None);
        // Hidden / NoDisplay are skipped.
        assert_eq!(
            session_name_from_desktop("[Desktop Entry]\nName=X\nExec=/x\nNoDisplay=true\n"),
            None
        );
        // A key outside [Desktop Entry] does not leak in.
        assert_eq!(
            session_name_from_desktop("[Other]\nName=Wrong\nExec=/x\n[Desktop Entry]\nExec=/y\n"),
            None
        );
        // The first unqualified Name wins; a localised Name[de] is ignored.
        assert_eq!(
            session_name_from_desktop("[Desktop Entry]\nName[de]=Sitzung\nName=Session\nExec=/s\n")
                .as_deref(),
            Some("Session")
        );
    }

    #[test]
    fn discovers_sorted_deduped_across_precedence() {
        let hi = tempfile::tempdir().unwrap();
        let lo = tempfile::tempdir().unwrap();
        // Same id in both dirs: the high-precedence one wins.
        write(hi.path(), "arlen.desktop", "[Desktop Entry]\nName=Arlen (local)\nExec=/a\n");
        write(lo.path(), "arlen.desktop", "[Desktop Entry]\nName=Arlen (system)\nExec=/a\n");
        write(lo.path(), "sway.desktop", "[Desktop Entry]\nName=Sway\nExec=/s\n");
        write(lo.path(), "notes.txt", "ignore me");
        write(lo.path(), "hidden.desktop", "[Desktop Entry]\nName=H\nExec=/h\nHidden=true\n");

        let out = discover_sessions(&[hi.path().to_path_buf(), lo.path().to_path_buf()]);
        // Sorted by name: "Arlen (local)" before "Sway"; the system arlen + the
        // hidden + the .txt are all excluded.
        assert_eq!(
            out.iter().map(|s| (s.id.as_str(), s.name.as_str())).collect::<Vec<_>>(),
            vec![("arlen", "Arlen (local)"), ("sway", "Sway")]
        );
    }

    #[test]
    fn missing_dirs_yield_nothing() {
        assert!(discover_sessions(&[PathBuf::from("/nonexistent/xyz")]).is_empty());
    }

    #[test]
    fn power_verb_maps_only_the_known_actions() {
        assert_eq!(power_verb("suspend"), Some("suspend"));
        assert_eq!(power_verb("reboot"), Some("reboot"));
        assert_eq!(power_verb("power-off"), Some("poweroff"));
        // Fail-closed: no passthrough of an arbitrary string to systemctl.
        assert_eq!(power_verb("poweroff"), None);
        assert_eq!(power_verb("--now; rm -rf /"), None);
        assert_eq!(power_verb(""), None);
    }
}
