//! The escalation entry point: the harness accepts an inbound session id so a
//! quick-ask started in the Waypointer overlay (`waypointer-ai-prompt.md` section 3,
//! Ctrl+J "continue in the agent") reopens as the SAME server-side session in the full
//! GUI. The id is carried on the LAUNCH, not the transcript - either `--session <id>`
//! or the `arlen://harness/session/<id>` deep-link form. The frontend reads it once on
//! mount via [`launch_session`] and loads that session instead of starting fresh.

/// Extract the launch session id from `args` (the process argv). Recognises the
/// `--session <id>` flag, its `--session=<id>` form, and an
/// `arlen://harness/session/<id>` deep-link argument, returning the FIRST match.
/// The value is validated to a safe id shape so a hostile launch arg can never carry a
/// path separator, whitespace or control char into the frontend's session loader;
/// `None` when absent or malformed. Pure over `args` so it is unit-tested without a
/// process launch.
pub fn launch_session_id(args: &[String]) -> Option<String> {
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let candidate = if let Some(rest) = arg.strip_prefix("--session=") {
            Some(rest.to_string())
        } else if arg == "--session" {
            it.next().cloned()
        } else {
            arg.strip_prefix("arlen://harness/session/").map(str::to_string)
        };
        if let Some(id) = candidate {
            if is_safe_session_id(&id) {
                return Some(id);
            }
        }
    }
    None
}

/// A session id is an opaque token the harness minted; accept only a conservative
/// shape (ASCII alphanumeric plus `-` `_` `.`, never `.`/`..`, bounded length) so an
/// inbound launch arg can never smuggle a path separator, whitespace or control char
/// into the session loader.
fn is_safe_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id != "."
        && id != ".."
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// The session id this harness was launched to open, if any (the Waypointer / Ctrl+J
/// escalation target). `None` for a normal launch; the frontend then starts fresh or
/// restores its last session as usual.
#[tauri::command]
pub fn launch_session() -> Option<String> {
    launch_session_id(&std::env::args().collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_the_session_flag_forms() {
        assert_eq!(
            launch_session_id(&v(&["harness", "--session", "abc123"])).as_deref(),
            Some("abc123")
        );
        assert_eq!(
            launch_session_id(&v(&["harness", "--session=abc123"])).as_deref(),
            Some("abc123")
        );
    }

    #[test]
    fn parses_the_deep_link_form() {
        assert_eq!(
            launch_session_id(&v(&["harness", "arlen://harness/session/s-42"])).as_deref(),
            Some("s-42")
        );
    }

    #[test]
    fn a_normal_launch_has_no_session() {
        assert_eq!(launch_session_id(&v(&["harness"])), None);
        assert_eq!(launch_session_id(&v(&["harness", "--other", "x"])), None);
    }

    #[test]
    fn a_hostile_id_is_rejected() {
        // Traversal, separators, whitespace, control - none may reach the loader.
        for bad in ["../../etc/passwd", "a/b", "a b", "..", ".", "", "a\nb"] {
            assert_eq!(
                launch_session_id(&v(&["h", "--session", bad])),
                None,
                "id {bad:?} must be rejected"
            );
        }
        // A missing value after the flag is None, never a panic.
        assert_eq!(launch_session_id(&v(&["h", "--session"])), None);
    }

    #[test]
    fn the_first_valid_session_wins() {
        assert_eq!(
            launch_session_id(&v(&["h", "--session", "first", "--session", "second"])).as_deref(),
            Some("first")
        );
    }
}
