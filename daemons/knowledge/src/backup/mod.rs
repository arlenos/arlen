/// Export/Import and backup for the Knowledge Graph.
///
/// - Export: JSON-LD + ZIP with manifest
/// - Import: Conflict resolution (skip/replace/merge)
/// - Snapshots: Snapper (Btrfs) integration
/// - Integrity: SQLite + graph consistency checks
///
/// See `docs/architecture/GRAPH-OPERATIONS.md` Sections 5-6.
///
/// **Nothing outside this module calls into it.** 858 lines and 24 tests, and
/// no socket op, timer or CLI path reaches any of it, so the daemon cannot
/// export, import, snapshot or integrity-check today. Stated here because the
/// module reads as a finished feature and there is no other way to tell.
///
/// Wiring it is a design step: export and import both need a caller-auth rule,
/// since a backup is by construction every entity the graph holds.

mod export;
mod import;
mod integrity;
mod snapshot;

pub use export::*;
pub use import::*;
pub use integrity::*;
pub use snapshot::*;
