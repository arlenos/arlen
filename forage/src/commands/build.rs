//! Build subcommand.
//!
//! The build pipeline (content-addressed store, fetch, sandbox, package, sign)
//! lands in forage-recipes.md R1. For now `build` validates the recipe against
//! the schema so the recipe is known-good before the pipeline exists.

use std::path::Path;
use std::process::exit;

use colored::Colorize;

use crate::commands::recipe;

/// Validate the recipe at `path`, then report that the build pipeline is
/// pending. Exits non-zero in all cases: if the recipe is invalid (via
/// [`recipe::validate`]), and also when it is valid, because no `.lunpkg` was
/// produced. A zero exit must mean a real, verified artifact, so scripts and CI
/// never mistake this for a completed build.
pub fn build(path: &Path) {
    recipe::validate(path);
    eprintln!(
        "{} the build pipeline is not yet implemented (forage-recipes.md R1); \
         the recipe above is schema-valid and ready for it, but no package was produced.",
        "note:".yellow().bold()
    );
    exit(1);
}
