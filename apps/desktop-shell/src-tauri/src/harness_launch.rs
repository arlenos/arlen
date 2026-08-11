//! Escalation from the Waypointer Ask mode to the full harness GUI
//! (waypointer-ai-prompt.md §3: "Ctrl+J continue in the agent"). The AI session
//! id, not the transcript, travels: both the launcher and the harness are thin
//! clients of `org.arlen.AIAgent1` with server-side session persistence, so
//! opening the harness on the same id lights up the full history, context and
//! tools without shuttling anything through the UI.

/// The harness binary name (on `PATH` after install).
const HARNESS_BIN: &str = "arlen-harness";

/// Whether a session id is safe to pass as a launch argument. A shell-local
/// mirror of the harness's own `is_safe_session_id` gate (the two apps share no
/// crate, so this is copied deliberately, like the capability reader): non-empty,
/// bounded, and restricted to a path-component-safe charset, so a hostile id can
/// never become a flag or escape into a path. Pure, so it is unit-tested.
fn is_safe_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id != "."
        && id != ".."
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Open the harness GUI on the given AI session id (the Waypointer `Ctrl+J`
/// escalation). Runs `arlen-harness --session <id>` under the shell's Wayland
/// environment; the harness resumes that server-side session. The id is passed as
/// a discrete argument (never shell-interpolated) and validated first, so a
/// malformed or hostile id is refused rather than launched.
///
/// **The spawn is not on a thread, and that is the fix rather than the style.**
/// It used to spawn from a detached thread and return `Ok(())` first, so a failure
/// could only ever reach the log - while this doc said, and the caller believed,
/// that it returned an error the launcher surfaces. `arlen-harness` is not in the
/// image today, so that path is the live one: Ctrl+J closed the overlay and
/// nothing opened, with nothing said. `Command::spawn` forks and execs without
/// waiting on the child, so there was never anything to move off the calling
/// thread; the thread bought nothing and cost the error.
///
/// A failure also raises a toast, because the caller deliberately swallows the
/// error: under vite there is no backend and every Ctrl+J would otherwise report
/// one. Telling the user from here keeps that swallow correct without leaving a
/// real failure silent.
#[tauri::command]
pub fn open_harness_session(app: tauri::AppHandle, id: String) -> Result<(), String> {
    if !is_safe_session_id(&id) {
        return Err("invalid session id".to_string());
    }
    let wayland_display = std::env::var("WAYLAND_DISPLAY").unwrap_or_default();
    match std::process::Command::new(HARNESS_BIN)
        .arg("--session")
        .arg(&id)
        .env("WAYLAND_DISPLAY", &wayland_display)
        .env("DISPLAY", "")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(_) => {
            log::info!("open_harness_session: launched harness on session");
            Ok(())
        }
        Err(e) => {
            log::error!("open_harness_session: spawn failed: {e}");
            crate::quick_actions::emit_toast(
                &app,
                crate::quick_actions::ToastKind::Error,
                format!("The assistant did not open: {e}"),
            );
            Err(format!("could not start {HARNESS_BIN}: {e}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ordinary_session_ids() {
        assert!(is_safe_session_id("abc123"));
        assert!(is_safe_session_id("2026-07-25_quick-ask.7"));
        assert!(is_safe_session_id(&"a".repeat(128)));
    }

    #[test]
    fn rejects_empty_and_over_length() {
        assert!(!is_safe_session_id(""));
        assert!(!is_safe_session_id(&"a".repeat(129)));
    }

    #[test]
    fn rejects_path_traversal_and_separators() {
        assert!(!is_safe_session_id("."));
        assert!(!is_safe_session_id(".."));
        assert!(!is_safe_session_id("../etc/passwd"));
        assert!(!is_safe_session_id("a/b"));
        assert!(!is_safe_session_id("a b"));
    }

    #[test]
    fn rejects_control_and_whitespace_chars() {
        // Control characters and whitespace must never reach argv. A leading
        // dash IS allowed (the charset mirrors the harness, whose ids may be
        // dash-bearing UUIDs); it is harmless because the id is consumed
        // positionally after `--session`, never as its own flag.
        assert!(!is_safe_session_id("a\nb"));
        assert!(!is_safe_session_id("a\0b"));
        assert!(!is_safe_session_id("a\tb"));
        assert!(is_safe_session_id("-dash-leading-is-fine"));
    }
}
