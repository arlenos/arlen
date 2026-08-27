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

/// The scoped one-shot directory a cross-app context inject is written into.
///
/// MUST match the terminal's `inject_dir` (`apps/terminal/src-tauri/src/lib.rs`).
/// Two copies of one convention, deliberately: a shared crate for one path helper
/// would be more machinery than the agreement is worth, and the agreement is
/// stated on both sides. If either moves, the other stops finding payloads.
fn inject_dir() -> Option<std::path::PathBuf> {
    if let Some(rt) = std::env::var_os("XDG_RUNTIME_DIR") {
        return Some(std::path::PathBuf::from(rt).join("arlen").join("inject"));
    }
    let home = std::env::var_os("HOME")?;
    Some(std::path::PathBuf::from(home).join(".cache").join("arlen").join("inject"))
}

/// The payload path this harness was launched to read, if any.
///
/// The other half of the cross-app scoped-context inject (`terminal.md` 4.11): the
/// terminal writes a 0600 markdown payload of one finished block and launches
/// `arlen-harness --inject <path>`. Until 27 August nothing here read that flag,
/// so the terminal's "send this to the assistant" wrote the file, opened the
/// harness, and the harness showed nothing while the payload stayed in the
/// runtime directory unread.
///
/// **The path arrives on argv, from whichever process did the launching**, so it
/// is confined to `dir` rather than taken at face value: the file must sit
/// directly in the inject directory, under a plain name. Without that, any
/// launcher could point the harness at `~/.ssh/id_rsa` and have it read into a
/// prompt. Pure over `args` and `dir` so both halves are unit-tested.
pub fn launch_inject_path(args: &[String], dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let candidate = if let Some(rest) = arg.strip_prefix("--inject=") {
            Some(rest.to_string())
        } else if arg == "--inject" {
            it.next().cloned()
        } else {
            None
        };
        let Some(raw) = candidate else { continue };
        let path = std::path::Path::new(&raw);
        // A plain file name directly under the agreed directory. `parent` and
        // `file_name` together refuse `..`, a nested path and a bare directory,
        // and the comparison is against the caller-supplied `dir` rather than the
        // environment so a test can hand in a real one.
        let Some(name) = path.file_name() else { continue };
        if path.parent() != Some(dir) || name.to_string_lossy().starts_with('.') {
            continue;
        }
        return Some(dir.join(name));
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

/// The block the harness was launched to receive, read once and then removed.
///
/// Returns the payload through the same capped reader an `@`-mention uses, so a
/// huge or non-UTF-8 file is bounded and lossy-decoded rather than trusted, and
/// then deletes it: the payload is a one-shot grant for this launch, and one left
/// behind is both a second chance to read it and litter in the runtime directory.
/// `None` for an ordinary launch, and also when the file is gone - a second call
/// in the same session must not re-insert the block.
#[tauri::command]
pub async fn launch_inject() -> Option<crate::mention::MentionContent> {
    let dir = inject_dir()?;
    let args: Vec<String> = std::env::args().collect();
    let path = launch_inject_path(&args, &dir)?;
    let content = crate::mention::read_mention_file(path.to_string_lossy().into_owned())
        .await
        .ok()?;
    // Best-effort: a payload that was read but could not be removed is still
    // better delivered than withheld, and the directory is per-user and 0700.
    let _ = std::fs::remove_file(&path);
    Some(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    /// The inject dir the tests hand in, so nothing depends on the environment.
    fn dir() -> std::path::PathBuf {
        std::path::PathBuf::from("/run/user/1000/arlen/inject")
    }

    #[test]
    fn an_inject_payload_in_the_agreed_directory_is_accepted() {
        let d = dir();
        assert_eq!(
            launch_inject_path(&v(&["arlen-harness", "--inject", "/run/user/1000/arlen/inject/b-1.md"]), &d),
            Some(d.join("b-1.md"))
        );
        assert_eq!(
            launch_inject_path(&v(&["arlen-harness", "--inject=/run/user/1000/arlen/inject/b-2.md"]), &d),
            Some(d.join("b-2.md"))
        );
        assert_eq!(launch_inject_path(&v(&["arlen-harness"]), &d), None);
    }

    #[test]
    fn a_path_outside_the_agreed_directory_is_refused() {
        let d = dir();
        // The argument comes from whoever launched us, so each of these is a
        // launcher asking the harness to read something it was not handed.
        for hostile in [
            "/home/u/.ssh/id_rsa",
            "/run/user/1000/arlen/inject/../../../etc/shadow",
            "/run/user/1000/arlen/inject/nested/b.md",
            "/run/user/1000/arlen/inject",
            "b.md",
        ] {
            assert_eq!(
                launch_inject_path(&v(&["arlen-harness", "--inject", hostile]), &d),
                None,
                "{hostile} must not be read"
            );
        }
    }

    #[test]
    fn a_session_launch_carries_no_inject_and_the_other_way_round() {
        let d = dir();
        let session = v(&["arlen-harness", "--session", "abc"]);
        assert_eq!(launch_inject_path(&session, &d), None);
        let inject = v(&["arlen-harness", "--inject", "/run/user/1000/arlen/inject/b.md"]);
        assert_eq!(launch_session_id(&inject), None);
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
