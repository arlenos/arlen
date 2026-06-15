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

/// The launchable sessions. Stub: fails until wired to greetd.
#[tauri::command]
fn greeter_sessions() -> Result<Vec<Session>, String> {
    Err("greeter backend not connected".to_string())
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

/// A power action from the login screen. Stub: the coder calls
/// `org.arlen.Power1` (or the same systemctl/loginctl the shell uses).
#[tauri::command]
fn greeter_power(_action: String) -> Result<(), String> {
    Err("greeter backend not connected".to_string())
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
