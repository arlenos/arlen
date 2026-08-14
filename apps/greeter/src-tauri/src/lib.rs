//! Arlen greeter host: the thin Tauri command layer over `arlen-greeter-core`.
//! The login logic (Wayland-session discovery, `/etc/passwd` profile enumeration +
//! avatars, the greetd auth conversation, the power-action map) lives in the core
//! crate, which is unit-tested in CI; this file is only the IPC seam the Svelte UI
//! drives, plus the little I/O (read `/etc/passwd`, dial `GREETD_SOCK`, spawn
//! `systemctl`) that a pure core cannot hold.


mod wallpaper;

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

/// What this login screen remembers about itself.
///
/// The greeter's own state, not any user's: *this login screen was last used
/// with a screen reader*. Read at startup so somebody who cannot see it can
/// reach the prompt without finding the toggle again at every boot.
///
/// An unreadable file answers with the default rather than failing: a login
/// screen that will not draw because of one boolean is worse for everyone than
/// one that draws with the toggle off, and the toggle is on screen.
///
/// Returns the BARE boolean, not the struct. serde serialises the struct's
/// field as `screen_reader` while the caller would naturally read
/// `screenReader`, and a mismatch there is not a type error in either language
/// - it is `undefined`, which is falsy, so the login screen would quietly
/// forget every setting and nothing would say why. One value has no shape to
/// get wrong.
#[tauri::command]
fn greeter_a11y_get() -> bool {
    match core::a11y_state::load_in(&core::a11y_state::state_dir()) {
        Ok(state) => state.screen_reader,
        Err(e) => {
            eprintln!("greeter: cannot read the remembered accessibility state ({e})");
            false
        }
    }
}

/// Remember the toggle for the next boot.
///
/// Called the moment it is operated, not on a successful login: somebody who
/// switches the reader on and then mistypes their password still needs it when
/// they try again.
#[tauri::command]
fn greeter_a11y_set(screen_reader: bool) -> Result<(), String> {
    core::a11y_state::store_in(
        &core::a11y_state::state_dir(),
        core::a11y_state::GreeterA11y { screen_reader },
    )
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
    // The login screen's screen-reader toggle, handed to the session it starts -
    // but ONLY when somebody operated it at this login. `None` means the toggle
    // sat at whatever the login screen remembered and nobody touched it, which
    // the session reads as "keep what your own config says" rather than "off".
    screen_reader: Option<bool>,
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
    core::run_login(stream, &profile_id, &secret, cmd, core::session_env(&session_id, screen_reader))?;
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
    // Dependencies at warn, this app at info. A blanket `info` also turns on
    // zbus, which logs D-Bus handshake frames WITH their message bytes - and a
    // message body is user content: file paths, query strings, notification
    // text. At info that lands in the journal, readable by anything with
    // journal access and covered by no capability grant, which undoes in a log
    // line what the graph's scoping is for. A byte trace stays available as
    // `RUST_LOG=zbus=trace`, deliberately, rather than by default.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,arlen_greeter_lib=info")).init();

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            wallpaper::greeter_wallpaper,
            greeter_profiles,
            greeter_sessions,
            greeter_authenticate,
            greeter_factor_begin,
            greeter_power,
            greeter_a11y_get,
            greeter_a11y_set
        ])
        .run(tauri::generate_context!())
        .expect("error while running arlen-greeter");
}
