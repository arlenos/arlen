//! Arlen greeter host: the thin Tauri command layer over `arlen-greeter-core`.
//! The login logic (Wayland-session discovery, `/etc/passwd` profile enumeration +
//! avatars, the greetd auth conversation, the power-action map) lives in the core
//! crate, which is unit-tested in CI; this file is only the IPC seam the Svelte UI
//! drives, plus the little I/O (read `/etc/passwd`, dial `GREETD_SOCK`, spawn
//! `systemctl`) that a pure core cannot hold.

use arlen_greeter_core as core;
use arlen_greeter_core::{Profile, Session};
use arlen_lock_auth::GREETD_SOCK_ENV;
use std::os::unix::net::UnixStream;
use std::path::Path;

/// The human login profiles, from `/etc/passwd`, each enriched with its
/// AccountsService avatar. Fails closed if the account list cannot be read.
#[tauri::command]
fn greeter_profiles() -> Result<Vec<Profile>, String> {
    let passwd = std::fs::read_to_string("/etc/passwd")
        .map_err(|e| format!("cannot read the account list: {e}"))?;
    let mut profiles = core::parse_login_accounts(&passwd, core::UID_MIN, core::UID_MAX);
    let icons = Path::new(core::ACCOUNTS_ICONS_DIR);
    for p in &mut profiles {
        p.avatar_url = core::resolve_avatar(icons, &p.id);
    }
    Ok(profiles)
}

/// The launchable Wayland sessions, always at least the Arlen fallback.
#[tauri::command]
fn greeter_sessions() -> Result<Vec<Session>, String> {
    let mut sessions = core::discover_sessions(&core::wayland_session_dirs());
    if sessions.is_empty() {
        sessions.push(core::fallback_session());
    }
    Ok(sessions)
}

/// Password authentication through greetd, starting the chosen session on success.
/// The profile is cross-checked against the offered accounts (an authorization
/// boundary, not just the picker), and an unknown session id is refused before
/// greetd is touched.
#[tauri::command]
fn greeter_authenticate(
    profile_id: String,
    secret: String,
    session_id: String,
) -> Result<serde_json::Value, String> {
    let passwd = std::fs::read_to_string("/etc/passwd")
        .map_err(|_| "login is not reachable (account list unavailable)".to_string())?;
    if !core::parse_login_accounts(&passwd, core::UID_MIN, core::UID_MAX)
        .iter()
        .any(|p| p.id == profile_id)
    {
        return Err("unknown profile".to_string());
    }
    let cmd = core::session_command(&core::wayland_session_dirs(), &session_id)
        .ok_or_else(|| format!("unknown session: {session_id}"))?;
    let sock = std::env::var(GREETD_SOCK_ENV)
        .map_err(|_| "login is not reachable (greetd socket unavailable)".to_string())?;
    let stream = UnixStream::connect(&sock).map_err(|e| format!("cannot reach greetd: {e}"))?;
    core::run_login(stream, &profile_id, &secret, cmd, core::session_env(&session_id))?;
    Ok(serde_json::json!({ "ok": true }))
}

/// Begin a hardware-factor login (FIDO2 / TPM2). Stub: wired to the lock-auth
/// factor abstraction when the hardware-factor backends land.
#[tauri::command]
fn greeter_factor_begin(_profile_id: String, _factor: String) -> Result<serde_json::Value, String> {
    Err("greeter backend not connected".to_string())
}

/// A power action from the login screen: `systemctl <verb>` for the three mapped
/// actions, anything else refused.
#[tauri::command]
fn greeter_power(action: String) -> Result<(), String> {
    let verb = core::power_verb(&action).ok_or_else(|| format!("unknown power action: {action}"))?;
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
