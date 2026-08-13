//! Stamping the identity of what the session starts.
//!
//! The session is the root of trust for a user session: it is what the login path
//! starts, exactly once, and everything else in the session descends from it. So it
//! is also the party that can say what those descendants ARE - and saying it is
//! what makes their identity survive a reader that cannot look for itself.
//!
//! # Why a stamp rather than letting each daemon be read
//!
//! A daemon that authenticates a peer would rather just resolve it: read
//! `/proc/<pid>/exe`, map the path to an app id, done. That stops working in the
//! units most worth hardening - `ProtectSystem=strict` refuses a same-uid peer's
//! exe link, which is the `readlinkat(exe): Permission denied` the undo signer hits
//! on every boot. A stamp registered at spawn is a fact recorded while it was still
//! readable, and answers later by lookup instead.
//!
//! # Where the id comes from, and why not from here
//!
//! From the binary, through the same [`path_to_app_id`] every other resolver uses.
//! Not from a name this module chooses: two resolvers naming one daemon differently
//! is how a profile lookup silently misses, and a miss answers "no grants", which
//! reads as correctly-locked-down rather than misconfigured. Measured on the unit
//! table, where `arlen-ai-engine-daemon` is `ai-agent` and a name-derived guess
//! would have been wrong.
//!
//! Best-effort throughout, like the launcher's: a broker that is down or a program
//! that cannot be resolved must never stop the session from starting. The child
//! then resolves the old way, which is exactly the state we are improving on.

use std::path::{Path, PathBuf};

/// The absolute path of `program`, resolved the way a shell would.
///
/// The session spawns its children by bare name, and the stamp needs the binary
/// they will run - resolved BEFORE the spawn, deliberately. Reading the child's
/// own `/proc/<pid>/exe` afterwards looks more direct and races: `systemd-cat`
/// execs into the program, so a read that wins the race sees `systemd-cat` and
/// stamps the wrong thing.
pub fn resolve_program(program: &str, path_var: &str) -> Option<PathBuf> {
    if program.contains('/') {
        let p = PathBuf::from(program);
        return p.is_file().then_some(p);
    }
    path_var
        .split(':')
        .filter(|d| !d.is_empty())
        .map(|d| Path::new(d).join(program))
        .find(|p| p.is_file())
}

/// The app id to stamp for `program`, or `None` when it resolves to nothing we can
/// name - in which case the child is left to the older resolvers rather than given
/// an id this module invented.
pub fn app_id_for_program(program: &str, path_var: &str) -> Option<String> {
    let path = resolve_program(program, path_var)?;
    arlen_permissions::identity::path_to_app_id(&path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_name_is_resolved_against_the_path_in_order() {
        let tmp = tempfile::tempdir().unwrap();
        let first = tmp.path().join("first");
        let second = tmp.path().join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(second.join("prog"), "x").unwrap();

        let path_var = format!("{}:{}", first.display(), second.display());
        assert_eq!(
            resolve_program("prog", &path_var),
            Some(second.join("prog")),
            "the first directory that HAS it wins, not the first directory"
        );
        // Both copies present: the earlier directory wins, as a shell would.
        std::fs::write(first.join("prog"), "x").unwrap();
        assert_eq!(resolve_program("prog", &path_var), Some(first.join("prog")));
    }

    #[test]
    fn a_program_that_is_not_there_resolves_to_nothing() {
        assert_eq!(resolve_program("no-such-program", "/nonexistent"), None);
        // And an empty PATH element is skipped rather than turned into a relative
        // lookup against the current directory.
        assert_eq!(resolve_program("sh", ""), None);
    }

    #[test]
    fn an_unnameable_binary_is_left_to_the_older_resolvers() {
        // No id is better than one this module invented: a stamp that disagrees
        // with the binary route makes a profile lookup miss, and a miss reads as
        // correctly-locked-down.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("odd"), "x").unwrap();
        assert_eq!(
            app_id_for_program("odd", &tmp.path().display().to_string()),
            None
        );
    }
}
