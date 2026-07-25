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
/// interactive terminal (the binary passes `stdout().is_terminal()` or equivalent);
/// a non-interactive run is treated as scripted and hard-unlinks.
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

    #[test]
    fn purge_always_hard_unlinks() {
        assert_eq!(route_delete(&inv(&["--purge", "secret"]), true), DeleteMode::Unlink);
        assert_eq!(route_delete(&inv(&["--purge", "secret"]), false), DeleteMode::Unlink);
    }
}
