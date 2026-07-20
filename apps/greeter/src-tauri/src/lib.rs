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

use arlen_lock_auth::{AuthStep, GreetdClient, GREETD_SOCK_ENV};
use serde::Serialize;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

/// A hard ceiling on the greetd auth exchange, so a misbehaving or hostile PAM
/// conversation (endless prompts) fails closed instead of spinning. A password
/// login is one secret prompt; the slack covers an info message or two.
const MAX_AUTH_STEPS: usize = 8;

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

/// The launch command for the plain Arlen fallback session (matches the id
/// [`fallback_session`] offers), so a minimal system with no `.desktop` sessions
/// installed still logs in. An absolute path, so it does not depend on greetd's
/// PATH.
fn fallback_command() -> Vec<String> {
    vec!["/usr/bin/arlen-session".to_string()]
}

/// Extract the `Exec=` launch command from a `.desktop` file's `[Desktop Entry]`,
/// split into an argv vector. This is what greetd's `StartSession` runs. Uses the
/// SAME tolerant `key = value` split + group handling as [`session_name_from_desktop`]
/// (so a session the picker lists is one that actually launches), and returns
/// `None` for a `NoDisplay`/`Hidden` entry so a hidden session is not launched by
/// id either. The argv split is on ASCII whitespace: session entries carry a plain
/// command (no `.desktop` field codes like `%f`, which are for file-opening app
/// launchers, not sessions). `None` when there is no non-empty `Exec`.
fn session_exec_from_desktop(contents: &str) -> Option<Vec<String>> {
    let mut in_entry = false;
    let mut exec: Option<Vec<String>> = None;
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
            ("Exec", v) if exec.is_none() => {
                let argv: Vec<String> = v.split_whitespace().map(str::to_string).collect();
                if !argv.is_empty() {
                    exec = Some(argv);
                }
            }
            ("NoDisplay", "true") | ("Hidden", "true") => return None,
            _ => {}
        }
    }
    exec
}

/// Resolve a session id to the argv greetd should `StartSession`. Searches `dirs`
/// (highest precedence first) for `<id>.desktop` and returns its `Exec` argv; the
/// fallback id resolves to [`fallback_command`] even with nothing installed. `None`
/// when the id is neither the fallback nor a discoverable session, so the caller
/// refuses to launch an unknown id rather than guessing.
fn session_command(dirs: &[PathBuf], id: &str) -> Option<Vec<String>> {
    // Guard the interpolation into a path: an id with a separator or traversal
    // must never escape the session dirs.
    if id.is_empty() || id.contains('/') || id.contains("..") {
        return None;
    }
    for dir in dirs {
        let path = dir.join(format!("{id}.desktop"));
        if let Ok(contents) = std::fs::read_to_string(&path) {
            if let Some(argv) = session_exec_from_desktop(&contents) {
                return Some(argv);
            }
        }
    }
    if id == fallback_session().id {
        return Some(fallback_command());
    }
    None
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

/// The default human-login UID window (Debian `login.defs` UID_MIN/UID_MAX). A
/// service/system account sits below this; `nobody` sits above it.
const UID_MIN: u32 = 1000;
const UID_MAX: u32 = 60000;

/// A shell that means "this account cannot log in interactively", so it is not a
/// human profile even if its UID falls in the login window. An EMPTY shell field
/// is NOT excluded: by passwd convention an empty shell defaults to `/bin/sh`, a
/// real interactive login, so such an account is a human profile.
fn is_login_shell(shell: &str) -> bool {
    !matches!(
        shell,
        "/usr/sbin/nologin"
            | "/sbin/nologin"
            | "/usr/bin/nologin"
            | "/bin/false"
            | "/usr/bin/false"
            | "/bin/sync"
    )
}

/// Parse the human-login accounts out of `/etc/passwd` contents: the `name:x:uid:
/// gid:gecos:home:shell` records whose uid is in `[min_uid, max_uid]` and whose
/// shell is a real login shell. The display name is the GECOS full-name field (up
/// to the first comma) when set, else the username. Every profile is `standard`
/// with the password factor; hardware-factor enrolment (fido2/tpm2) and the
/// last-used pre-selection are later refinements, not derivable from passwd.
/// Sorted by display name for a stable picker. Pure, so it is unit-tested.
fn parse_login_accounts(passwd: &str, min_uid: u32, max_uid: u32) -> Vec<Profile> {
    let mut out: Vec<Profile> = passwd
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            if f.len() < 7 {
                return None;
            }
            let (name, uid_s, gecos, shell) = (f[0], f[2], f[4], f[6]);
            let uid: u32 = uid_s.parse().ok()?;
            if uid < min_uid || uid > max_uid || !is_login_shell(shell) || name.is_empty() {
                return None;
            }
            // The GECOS full-name is user-settable via `chfn`; strip control chars
            // before it reaches the webview (defense-in-depth beside Svelte's HTML
            // escaping). An all-stripped/empty name falls back to the username.
            let raw = gecos.split(',').next().filter(|s| !s.is_empty()).unwrap_or(name);
            let display: String = raw.chars().filter(|c| !c.is_control()).collect();
            let display = if display.is_empty() { name.to_string() } else { display };
            Some(Profile {
                id: name.to_string(),
                name: display,
                avatar_url: None,
                kind: "standard".to_string(),
                last_used: false,
                factors: vec!["password".to_string()],
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// The AccountsService avatar directory: one image per username, the standard
/// place a display/login manager reads a user's picture from (it is root-owned +
/// world-readable, so the greeter can read it before any session).
const ACCOUNTS_ICONS_DIR: &str = "/var/lib/AccountsService/icons";

/// Cap on an avatar image (a login avatar is small; a larger file is skipped
/// rather than base64'd into the reply).
const MAX_AVATAR_BYTES: u64 = 2 * 1024 * 1024;

/// Resolve a user's avatar to a `data:` URL the webview can render inline, or
/// `None` for the initials fallback. Reads `<icons_dir>/<username>`, sniffs PNG vs
/// JPEG by magic bytes (an unknown type is skipped, not mislabelled), and base64s
/// it. The username is guarded against a path escape, and an oversized file is
/// skipped. Pure over the dir, so it is unit-tested with a fixture.
fn resolve_avatar(icons_dir: &Path, username: &str) -> Option<String> {
    use base64::Engine;
    use std::io::Read;
    if username.is_empty() || username.contains('/') || username.contains("..") {
        return None;
    }
    let path = icons_dir.join(username);
    // symlink_metadata does NOT follow a final symlink, and is_file() rejects a
    // symlink / fifo / device / dir - so a planted symlink cannot redirect the read
    // to an arbitrary root-readable file (disclosure) or an endless special file
    // (DoS). The size is then bounded by the READ itself (`.take`), not by trusting
    // a metadata length that a growing or special file could lie about.
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MAX_AVATAR_BYTES {
        return None;
    }
    let file = std::fs::File::open(&path).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_AVATAR_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_AVATAR_BYTES {
        return None;
    }
    let mime = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else {
        return None;
    };
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Some(format!("data:{mime};base64,{b64}"))
}

/// The human login profiles the greeter offers, read from `/etc/passwd` (a file
/// the greeter is allowed even before a session), each enriched with its
/// AccountsService avatar when one exists. Fails closed if the account list cannot
/// be read, which the UI renders as "login is not reachable" rather than a
/// fake-empty list.
#[tauri::command]
fn greeter_profiles() -> Result<Vec<Profile>, String> {
    let passwd = std::fs::read_to_string("/etc/passwd")
        .map_err(|e| format!("cannot read the account list: {e}"))?;
    let mut profiles = parse_login_accounts(&passwd, UID_MIN, UID_MAX);
    let icons = Path::new(ACCOUNTS_ICONS_DIR);
    for p in &mut profiles {
        p.avatar_url = resolve_avatar(icons, &p.id);
    }
    Ok(profiles)
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

/// The environment greetd should add when starting the session. greetd + pam_
/// systemd populate most of it; this pins the session-type hints so a Wayland
/// compositor comes up correctly and the session is tagged with its id.
fn session_env(session_id: &str) -> Vec<String> {
    vec![
        "XDG_SESSION_TYPE=wayland".to_string(),
        format!("XDG_SESSION_DESKTOP={session_id}"),
        format!("XDG_CURRENT_DESKTOP={session_id}"),
    ]
}

/// Drive the greetd auth conversation to completion over `stream` and, on success,
/// start `cmd`. Generic over the stream so it is tested against an in-process mock
/// greetd. The policy: answer the first hidden prompt with `secret`, acknowledge
/// info/error messages and any further prompt with an empty response (the greeter
/// holds no other credential), stop on `Authenticated` (then `StartSession`) or
/// `Failed`, and cap the exchange at [`MAX_AUTH_STEPS`] so it always terminates.
/// The secret is never logged and never leaves this function.
fn run_login<S: Read + Write>(
    stream: S,
    username: &str,
    secret: &str,
    cmd: Vec<String>,
    env: Vec<String>,
) -> Result<(), String> {
    let mut client = GreetdClient::new(stream);
    let mut step = client
        .create_session(username)
        .map_err(|e| format!("greetd create_session failed: {e}"))?;
    let mut answered_secret = false;
    for _ in 0..MAX_AUTH_STEPS {
        match step {
            AuthStep::Authenticated => {
                return client
                    .start_session(cmd, env)
                    .map_err(|e| format!("greetd start_session failed: {e}"));
            }
            AuthStep::Failed { description } => {
                let _ = client.cancel();
                // Generic on purpose: greetd/PAM's description can differentiate
                // "no such user" from "wrong password", which is a username-
                // enumeration oracle. The greeter surfaces one failure to the UI.
                log::debug!("greetd auth failed: {description}");
                return Err("authentication failed".to_string());
            }
            AuthStep::Prompt { secret: true, .. } if !answered_secret => {
                answered_secret = true;
                step = client
                    .post_response(Some(secret.to_string()))
                    .map_err(|e| format!("greetd auth failed: {e}"))?;
            }
            // A further prompt (or a message) the greeter cannot answer: acknowledge
            // with an empty response and let greetd decide. A second secret prompt
            // means the credential was not accepted as the answer.
            AuthStep::Prompt { .. } | AuthStep::Message { .. } => {
                step = client
                    .post_response(None)
                    .map_err(|e| format!("greetd auth failed: {e}"))?;
            }
        }
    }
    let _ = client.cancel();
    Err("authentication did not complete".to_string())
}

/// Password authentication: run the greetd conversation for `profile_id` with
/// `secret`, and on success start the chosen `session_id`. The session id is
/// resolved to its launch command locally (a filesystem read the greeter is
/// allowed) and an unknown id is refused before touching greetd. Returns an
/// `AuthResult`-shaped `{ ok: true }` on success; a wrong credential or any greetd
/// error surfaces as `Err`, which the frontend renders as a failed login.
#[tauri::command]
fn greeter_authenticate(
    profile_id: String,
    secret: String,
    session_id: String,
) -> Result<serde_json::Value, String> {
    // The UID/shell account filter is an AUTHORIZATION boundary, not just what the
    // picker shows: a caller of this IPC command must not drive a PAM auth for an
    // account the greeter would never offer (root, a service account, a uid outside
    // the login window). PAM still gates the credential, but the greeter enforces
    // its own account policy here rather than trusting the frontend.
    let passwd = std::fs::read_to_string("/etc/passwd")
        .map_err(|_| "login is not reachable (account list unavailable)".to_string())?;
    if !parse_login_accounts(&passwd, UID_MIN, UID_MAX).iter().any(|p| p.id == profile_id) {
        return Err("unknown profile".to_string());
    }
    let cmd = session_command(&wayland_session_dirs(), &session_id)
        .ok_or_else(|| format!("unknown session: {session_id}"))?;
    let sock = std::env::var(GREETD_SOCK_ENV)
        .map_err(|_| "login is not reachable (greetd socket unavailable)".to_string())?;
    let stream =
        UnixStream::connect(&sock).map_err(|e| format!("cannot reach greetd: {e}"))?;
    run_login(stream, &profile_id, &secret, cmd, session_env(&session_id))?;
    Ok(serde_json::json!({ "ok": true }))
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

    fn write(dir: &std::path::Path, file: &str, body: &str) {
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
    fn session_command_resolves_exec_and_guards_the_id() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "sway.desktop", "[Desktop Entry]\nName=Sway\nExec=/usr/bin/sway --unsupported\n");
        let dirs = [dir.path().to_path_buf()];

        assert_eq!(
            session_command(&dirs, "sway"),
            Some(vec!["/usr/bin/sway".to_string(), "--unsupported".to_string()])
        );
        // The fallback id resolves even with nothing on disk for it.
        assert_eq!(session_command(&dirs, "arlen"), Some(fallback_command()));
        // Unknown id -> None (refuse to launch a guess).
        assert_eq!(session_command(&dirs, "ghost"), None);
        // Path-escape guards: a separator or traversal never reaches the fs join.
        assert_eq!(session_command(&dirs, "../etc/passwd"), None);
        assert_eq!(session_command(&dirs, "a/b"), None);
        assert_eq!(session_command(&dirs, ""), None);
    }

    /// A one-shot mock greetd on `srv`: expect CreateSession, send a Secret
    /// prompt, expect the posted secret, then answer `auth_ok` (Success) or a
    /// Failed(AuthError). On success also expect + accept StartSession. Runs on a
    /// thread so the blocking client conversation on the other end does not
    /// deadlock. Returns the secret greetd received and the StartSession cmd.
    fn mock_greetd(
        srv: UnixStream,
        auth_ok: bool,
    ) -> std::thread::JoinHandle<(Option<String>, Option<Vec<String>>)> {
        use greetd_ipc::{codec::SyncCodec, AuthMessageType, ErrorType, Request, Response};
        std::thread::spawn(move || {
            let mut s = srv;
            let mut got_cmd = None;
            match Request::read_from(&mut s).unwrap() {
                Request::CreateSession { .. } => {}
                other => panic!("expected CreateSession, got {other:?}"),
            }
            Response::AuthMessage {
                auth_message_type: AuthMessageType::Secret,
                auth_message: "Password:".to_string(),
            }
            .write_to(&mut s)
            .unwrap();
            let got_secret = match Request::read_from(&mut s).unwrap() {
                Request::PostAuthMessageResponse { response } => response,
                other => panic!("expected PostAuthMessageResponse, got {other:?}"),
            };
            if auth_ok {
                Response::Success.write_to(&mut s).unwrap();
                match Request::read_from(&mut s).unwrap() {
                    Request::StartSession { cmd, .. } => got_cmd = Some(cmd),
                    other => panic!("expected StartSession, got {other:?}"),
                }
                Response::Success.write_to(&mut s).unwrap();
            } else {
                Response::Error {
                    error_type: ErrorType::AuthError,
                    description: "wrong password".to_string(),
                }
                .write_to(&mut s)
                .unwrap();
                // The client cancels a failed session; accept it if it arrives.
                let _ = Request::read_from(&mut s);
            }
            (got_secret, got_cmd)
        })
    }

    #[test]
    fn run_login_answers_the_secret_and_starts_the_session_on_success() {
        let (client, server) = UnixStream::pair().unwrap();
        let greetd = mock_greetd(server, true);
        let out = run_login(
            client,
            "alice",
            "hunter2",
            vec!["arlen-session".to_string()],
            vec!["XDG_SESSION_TYPE=wayland".to_string()],
        );
        let (secret, cmd) = greetd.join().unwrap();
        assert!(out.is_ok(), "{out:?}");
        assert_eq!(secret.as_deref(), Some("hunter2"));
        assert_eq!(cmd, Some(vec!["arlen-session".to_string()]));
    }

    #[test]
    fn run_login_surfaces_a_wrong_credential_as_an_error() {
        let (client, server) = UnixStream::pair().unwrap();
        let greetd = mock_greetd(server, false);
        let out = run_login(client, "alice", "bad", vec!["arlen-session".to_string()], vec![]);
        let (secret, cmd) = greetd.join().unwrap();
        assert!(out.is_err());
        // Generic message (no PAM-text enumeration oracle).
        assert_eq!(out.unwrap_err(), "authentication failed");
        assert_eq!(secret.as_deref(), Some("bad")); // greetd still received the attempt
        assert_eq!(cmd, None); // no session started
    }

    #[test]
    fn parse_login_accounts_keeps_only_human_logins() {
        let passwd = "\
root:x:0:0:root:/root:/bin/bash
daemon:x:1:1:daemon:/usr/sbin:/usr/sbin/nologin
alice:x:1000:1000:Alice Example,,,:/home/alice:/bin/bash
bob:x:1001:1001::/home/bob:/usr/bin/zsh
svc:x:1002:1002:service:/var/lib/svc:/usr/sbin/nologin
nobody:x:65534:65534:nobody:/nonexistent:/usr/sbin/nologin
";
        let out = parse_login_accounts(passwd, UID_MIN, UID_MAX);
        // root (uid 0) + the two nologin service accounts + nobody are excluded;
        // alice (GECOS name) and bob (no GECOS -> username) remain, name-sorted.
        assert_eq!(
            out.iter().map(|p| (p.id.as_str(), p.name.as_str())).collect::<Vec<_>>(),
            vec![("alice", "Alice Example"), ("bob", "bob")]
        );
        assert!(out.iter().all(|p| p.kind == "standard" && p.factors == vec!["password"]));
    }

    #[test]
    fn resolve_avatar_encodes_a_known_image_and_guards_the_name() {
        let dir = tempfile::tempdir().unwrap();
        // A minimal "PNG" (magic bytes + a byte) and a "JPEG".
        std::fs::write(dir.path().join("alice"), [0x89, b'P', b'N', b'G', 0x01]).unwrap();
        std::fs::write(dir.path().join("bob"), [0xFF, 0xD8, 0xFF, 0x02]).unwrap();
        std::fs::write(dir.path().join("carol"), b"not an image").unwrap();

        assert!(resolve_avatar(dir.path(), "alice").unwrap().starts_with("data:image/png;base64,"));
        assert!(resolve_avatar(dir.path(), "bob").unwrap().starts_with("data:image/jpeg;base64,"));
        // Unknown magic -> skipped (initials fallback), not mislabelled.
        assert_eq!(resolve_avatar(dir.path(), "carol"), None);
        // No file -> None.
        assert_eq!(resolve_avatar(dir.path(), "ghost"), None);
        // Path-escape guards.
        assert_eq!(resolve_avatar(dir.path(), "../etc/shadow"), None);
        assert_eq!(resolve_avatar(dir.path(), ""), None);
        // A symlink (even to a valid image) is rejected: no redirect to an
        // arbitrary root-readable file, no unbounded special-file read.
        #[cfg(unix)]
        {
            let target = dir.path().join("real.png");
            std::fs::write(&target, [0x89, b'P', b'N', b'G', 0x09]).unwrap();
            std::os::unix::fs::symlink(&target, dir.path().join("link")).unwrap();
            assert_eq!(resolve_avatar(dir.path(), "link"), None);
        }
    }

    #[test]
    fn is_login_shell_excludes_the_nologin_family() {
        assert!(is_login_shell("/bin/bash"));
        assert!(is_login_shell("/usr/bin/fish"));
        // Empty shell => /bin/sh by convention => a real login.
        assert!(is_login_shell(""));
        assert!(!is_login_shell("/usr/sbin/nologin"));
        assert!(!is_login_shell("/bin/false"));
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
