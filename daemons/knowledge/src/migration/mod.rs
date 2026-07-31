/// Schema migrations for the Knowledge Graph.
///
/// Migrations are TOML files named `{from}_to_{to}.toml` that describe
/// operations to transform entity schemas between versions. The runner
/// executes migrations in order with checkpoint support for resumability.
///
/// See `docs/architecture/SCHEMA-MIGRATIONS.md`.
///
/// **Nothing outside this module calls into it.** 1031 lines and 34 tests, and
/// no startup path runs the runner, so a schema version change does not migrate
/// anything today. Stated here because the module reads as a finished feature
/// and there is no other way to tell.
///
/// Wiring it is a design step: it needs a decision about when migrations run
/// (startup, before the first write of a changed schema, or on demand) and what
/// happens to a graph whose migration fails halfway, which is what the
/// checkpoint support is for.

mod checkpoint;
mod functions;
mod parser;
mod runner;

pub use checkpoint::*;
pub use functions::{apply_transform, list_functions};
pub use parser::*;
pub use runner::*;
