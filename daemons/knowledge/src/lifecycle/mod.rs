/// Entity lifecycle: soft delete, trash, restore, cleanup, staged uninstall.
///
/// See `docs/architecture/ENTITY-SCHEMA-SYSTEM.md` Section 6 and
/// `docs/architecture/GRAPH-OPERATIONS.md` Sections 3-4.
///
/// **Nothing outside this module calls into it.** The query builders are
/// complete and tested, 23 tests across the four files, but no socket op or
/// promotion path invokes them, so the daemon does not soft-delete, trash,
/// restore or stage an uninstall today. That is worth stating here because the
/// module reads as a finished feature and a reader has no other way to tell:
/// the only callers of `to_cypher` and friends are these files' own tests.
///
/// Wiring it is a design step rather than a missing line - it needs a mode byte
/// on the write socket, a caller-auth rule for who may trash another app's
/// entities, and a decision about whether cleanup runs on the retention timer or
/// on demand. Until then this is a mechanism waiting for its trigger, the same
/// shape as the executor and the canary before their call sites landed.

mod cleanup;
mod restore;
mod staged_uninstall;
mod trash;

pub use cleanup::*;
pub use restore::*;
pub use staged_uninstall::*;
pub use trash::*;
