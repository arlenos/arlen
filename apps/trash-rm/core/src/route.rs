//! Route a parsed delete to trash-first or hard-unlink.
//!
//! The Decision-2 routing (`compensable-action-history-plan.md` §4), settled here as
//! the coder's call: an INTERACTIVE delete is trash-first and reversible; a scripted
//! (non-interactive) `rm` keeps POSIX semantics and hard-unlinks, so a script relying
//! on the space being freed or on `rm`'s exact behaviour is never silently changed.
//! An explicit `--purge` always hard-unlinks (the escape hatch, in either context).
//! Note `-f` does NOT force a hard delete: `rm -rf foo` interactively is exactly the
//! catastrophic case the trash exists to make reversible.

use crate::parse::RmInvocation;

/// How a delete is carried out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteMode {
    /// Move the operands to the freedesktop trash, journaling a restorable inverse.
    Trash,
    /// Permanently unlink the operands (POSIX `rm` semantics).
    Unlink,
}

/// Decide the delete mode. `interactive_session` is whether the delete runs in an
/// interactive terminal; a non-interactive run is treated as scripted and
/// hard-unlinks.
///
/// The binary reads it from STDIN. This doc named stdout for an hour after the
/// binary stopped using it, which is the drift worth noting in a doc that exists
/// to tell a reader where the bool comes from: `rm old.log > out.txt` is a person
/// whose output happens to go elsewhere, and stdout calls that a script.
pub fn route_delete(inv: &RmInvocation, interactive_session: bool) -> DeleteMode {
    if inv.purge || !interactive_session {
        DeleteMode::Unlink
    } else {
        DeleteMode::Trash
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_rm_args;

    fn inv(v: &[&str]) -> RmInvocation {
        parse_rm_args(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn interactive_delete_is_trash_first() {
        // The catastrophic interactive case is exactly what trash makes reversible.
        assert_eq!(route_delete(&inv(&["-rf", "project"]), true), DeleteMode::Trash);
        assert_eq!(route_delete(&inv(&["notes.txt"]), true), DeleteMode::Trash);
    }

    #[test]
    fn scripted_delete_keeps_posix_unlink() {
        // Non-interactive (a script/pipeline) must not silently change semantics.
        assert_eq!(route_delete(&inv(&["-rf", "build"]), false), DeleteMode::Unlink);
    }

    /// The redirect case, which is why the caller tests STDIN rather than stdout.
    ///
    /// `rm old.log > out.txt` is a person at a keyboard whose OUTPUT happens to
    /// be going somewhere else. Read off stdout that looks exactly like a script
    /// and the file is hard-unlinked - the safety net gone at the moment somebody
    /// was typing. This function cannot see the difference (it is handed a bool),
    /// so the test pins the CONSEQUENCE: whatever the caller decides, an
    /// interactive answer must still reach the trash.
    #[test]
    fn an_interactive_session_reaches_the_trash_however_output_is_routed() {
        assert_eq!(route_delete(&inv(&["old.log"]), true), DeleteMode::Trash);
    }

    #[test]
    fn purge_always_hard_unlinks() {
        assert_eq!(route_delete(&inv(&["--purge", "secret"]), true), DeleteMode::Unlink);
        assert_eq!(route_delete(&inv(&["--purge", "secret"]), false), DeleteMode::Unlink);
    }
}
