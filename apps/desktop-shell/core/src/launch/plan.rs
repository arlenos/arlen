//! The one decision: does this launch go through `arlen-run`, and with what argv.
//!
//! Written twice already - the shell launcher's `launch_plan` and the per-app
//! Settings handoff's `handoff_command` - and both are outside the CI matrix, so
//! neither is checked where merges are gated. Two spellings of one rule is the
//! shape that had four call sites disagreeing about a build command this week,
//! and this rule decides whether an application runs inside its permission
//! profile or outside it.
//!
//! Pure: it produces the argv, the caller spawns it. That keeps the decision
//! testable without starting a process, which is why both existing copies had
//! already split it out - they just each split it out separately.

/// How to start an application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// Run this argv as it stands. Today's default.
    Direct(Vec<String>),
    /// Run this argv, which already begins with `arlen-run` and the app id, so
    /// the application starts under its permission profile.
    Confined(Vec<String>),
}

impl Launch {
    /// The argv to spawn, either way.
    pub fn argv(&self) -> &[String] {
        match self {
            Self::Direct(a) | Self::Confined(a) => a,
        }
    }
}

/// Why a launch could not be planned at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotLaunchable {
    /// No program to run. A caller that reaches this has an entry whose `Exec`
    /// was empty or expanded to nothing, which is a broken entry rather than an
    /// unconfined launch.
    EmptyArgv,
}

/// The name the confined path runs, and the argument contract `arlen-run`
/// parses: `arlen-run --app-id <id> -- <program> <args...>`.
const LAUNCHER: &str = "arlen-run";

/// Plan a launch.
///
/// `confined` is `shell.toml [launcher] confined`, default off, and off is
/// always [`Launch::Direct`] - so nothing about today's behaviour depends on
/// this being adopted.
///
/// **Confined but no app id yields `Direct`**, because `arlen-run` keys the
/// permission profile on the app id and has nothing to enforce without one. That
/// is the behaviour both existing copies already have, and it is the one line
/// the go-live has to revisit: a launch nobody can identify is exactly the
/// launch a confining system should refuse, and refusing it today would break
/// every application without a profile. Keeping it here means that decision is
/// made once, in a place with tests, rather than found twice later.
pub fn plan(
    confined: bool,
    app_id: Option<&str>,
    argv: &[String],
) -> Result<Launch, NotLaunchable> {
    if argv.is_empty() {
        return Err(NotLaunchable::EmptyArgv);
    }
    let id = app_id.filter(|id| !id.is_empty());
    match (confined, id) {
        (true, Some(id)) => {
            let mut out = vec![
                LAUNCHER.to_string(),
                "--app-id".to_string(),
                id.to_string(),
                "--".to_string(),
            ];
            out.extend_from_slice(argv);
            Ok(Launch::Confined(out))
        }
        _ => Ok(Launch::Direct(argv.to_vec())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn unconfined_runs_the_argv_as_it_stands() {
        let a = argv(&["prog", "--flag"]);
        assert_eq!(
            plan(false, Some("org.x.App"), &a),
            Ok(Launch::Direct(a.clone()))
        );
    }

    #[test]
    fn confined_prefixes_the_launcher_and_the_app_id() {
        assert_eq!(
            plan(true, Some("org.x.App"), &argv(&["prog", "--flag"])),
            Ok(Launch::Confined(argv(&[
                "arlen-run",
                "--app-id",
                "org.x.App",
                "--",
                "prog",
                "--flag"
            ])))
        );
    }

    /// The separator has to be there even with no arguments, or `arlen-run`
    /// cannot tell the program from its own options.
    #[test]
    fn the_separator_is_present_for_a_bare_program() {
        assert_eq!(
            plan(true, Some("a.b"), &argv(&["prog"])),
            Ok(Launch::Confined(argv(&[
                "arlen-run",
                "--app-id",
                "a.b",
                "--",
                "prog"
            ])))
        );
    }

    /// `arlen-run` keys the profile on the app id, so without one there is
    /// nothing for it to enforce. This is the line the go-live revisits.
    #[test]
    fn confined_without_an_app_id_falls_back_to_direct() {
        let a = argv(&["prog"]);
        assert_eq!(plan(true, None, &a), Ok(Launch::Direct(a.clone())));
        assert_eq!(plan(true, Some(""), &a), Ok(Launch::Direct(a)));
    }

    /// An argument that looks like a launcher option is data, and the separator
    /// is what keeps it that way.
    #[test]
    fn an_argument_shaped_like_a_launcher_flag_stays_after_the_separator() {
        let planned = plan(true, Some("a.b"), &argv(&["prog", "--app-id", "other"])).unwrap();
        let a = planned.argv();
        assert_eq!(a.iter().position(|s| s == "--"), Some(3));
        assert_eq!(&a[4..], ["prog", "--app-id", "other"]);
    }

    #[test]
    fn nothing_to_run_is_an_error_rather_than_an_unconfined_launch() {
        assert_eq!(plan(true, Some("a.b"), &[]), Err(NotLaunchable::EmptyArgv));
        assert_eq!(plan(false, None, &[]), Err(NotLaunchable::EmptyArgv));
    }

    #[test]
    fn the_argv_accessor_reaches_both_shapes() {
        assert_eq!(plan(false, None, &argv(&["p"])).unwrap().argv(), ["p"]);
        assert_eq!(
            plan(true, Some("a.b"), &argv(&["p"])).unwrap().argv()[0],
            "arlen-run"
        );
    }
}
