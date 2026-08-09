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
///
/// Both variants are an argv to spawn directly, with no shell in between. That
/// is worth saying because the shell launcher's unconfined path does not do
/// that today - see the note on [`plan`].
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
/// permission profile on the app id and has nothing to enforce without one.
///
/// **`has_profile` is the per-app rule, and it is what the flag means.** An
/// application with a profile is confined; one without runs exactly as it does
/// today. That is a deliberate choice over an all-or-nothing flag, which was
/// measured on 9 August against a real machine: of 230 `.desktop` entries, 170
/// carry an id `arlen-run` rejects as malformed (exit 64, mostly for having no
/// reverse-domain dot) and the remaining 60 have no profile (exit 65), so an
/// all-or-nothing flip stops all 230, silently, on the day it is turned on.
/// Under this rule those 230 are not a blocker - they are the unconfined set,
/// and it shrinks as profiles arrive.
///
/// The caller answers `has_profile`, because it is a filesystem question and
/// this is not a filesystem function. Ask it with `arlen_permissions::
/// profile_paths`, which returns every path the loader itself consults, so the
/// answer cannot drift from what `arlen-run` will do a moment later.
///
/// **Adopting this in the shell launcher changes behaviour, so it is not a
/// refactor.** That launcher's unconfined branch runs `sh -c "<the whole Exec
/// string>"`, while its confined branch splits the string and passes an argv to
/// `arlen-run`. So the same entry is interpreted two different ways depending on
/// a flag: a `Exec=env FOO=bar prog` or anything with `&&` in it works unconfined
/// and arrives as literal arguments confined. Whichever way that is settled, one
/// launch path means one interpretation, and the flip should not be the thing
/// that discovers the difference. The Settings handoff already spawns directly on
/// both branches and matches what this returns.
pub fn plan(
    confined: bool,
    app_id: Option<&str>,
    has_profile: bool,
    argv: &[String],
) -> Result<Launch, NotLaunchable> {
    if argv.is_empty() {
        return Err(NotLaunchable::EmptyArgv);
    }
    let id = app_id.filter(|id| !id.is_empty());
    match (confined && has_profile, id) {
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
            plan(false, Some("org.x.App"), true, &a),
            Ok(Launch::Direct(a.clone()))
        );
    }

    #[test]
    fn confined_prefixes_the_launcher_and_the_app_id() {
        assert_eq!(
            plan(true, Some("org.x.App"), true, &argv(&["prog", "--flag"])),
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
            plan(true, Some("a.b"), true, &argv(&["prog"])),
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
        assert_eq!(plan(true, None, true, &a), Ok(Launch::Direct(a.clone())));
        assert_eq!(plan(true, Some(""), true, &a), Ok(Launch::Direct(a)));
    }

    /// An argument that looks like a launcher option is data, and the separator
    /// is what keeps it that way.
    #[test]
    fn an_argument_shaped_like_a_launcher_flag_stays_after_the_separator() {
        let planned = plan(
            true,
            Some("a.b"),
            true,
            &argv(&["prog", "--app-id", "other"]),
        )
        .unwrap();
        let a = planned.argv();
        assert_eq!(a.iter().position(|s| s == "--"), Some(3));
        assert_eq!(&a[4..], ["prog", "--app-id", "other"]);
    }

    /// The per-app rule. An application the system holds no profile for is not
    /// routed through the launcher at all, so turning the flag on cannot stop it:
    /// `arlen-run` would refuse it (64 for a plain desktop-id, 65 for a missing
    /// profile) and the refusal would reach the user as an icon that does
    /// nothing.
    #[test]
    fn an_app_with_no_profile_is_not_confined_even_with_the_flag_on() {
        let a = argv(&["firefox"]);
        assert_eq!(
            plan(true, Some("firefox"), false, &a),
            Ok(Launch::Direct(a.clone()))
        );
        assert_eq!(
            plan(true, Some("dev.arlen.clock"), false, &a),
            Ok(Launch::Direct(a))
        );
    }

    #[test]
    fn nothing_to_run_is_an_error_rather_than_an_unconfined_launch() {
        assert_eq!(
            plan(true, Some("a.b"), true, &[]),
            Err(NotLaunchable::EmptyArgv)
        );
        assert_eq!(plan(false, None, true, &[]), Err(NotLaunchable::EmptyArgv));
    }

    #[test]
    fn the_argv_accessor_reaches_both_shapes() {
        assert_eq!(
            plan(false, None, true, &argv(&["p"])).unwrap().argv(),
            ["p"]
        );
        assert_eq!(
            plan(true, Some("a.b"), true, &argv(&["p"])).unwrap().argv()[0],
            "arlen-run"
        );
    }
}
