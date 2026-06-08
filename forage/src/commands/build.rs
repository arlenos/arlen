//! Build subcommand.
//!
//! The build crates (content-addressed store, fetch, sandbox, package, sign)
//! exist; wiring them into this command is gated on the pinned base platform
//! (roadmap D2) and on-kernel sandbox verification. Until that lands, `build`
//! validates the recipe against the schema so it is known-good and ready for the
//! pipeline.

use std::path::Path;
use std::process::exit;

use colored::Colorize;

use crate::commands::build_config::ForageBuildConfig;
use crate::commands::recipe;

/// Validate the recipe at `path`, then report that the build pipeline is
/// pending. Exits non-zero in all cases: if the recipe is invalid (via
/// [`recipe::validate`]), and also when it is valid, because no `.lunpkg` was
/// produced. A zero exit must mean a real, verified artifact, so scripts and CI
/// never mistake this for a completed build.
pub fn build(path: &Path) {
    recipe::validate(path);

    // Surface the one remaining prerequisite precisely: the build runs inside a
    // pinned base platform, and where that lives is deployment state read from
    // config. Report whether it is ready so the user knows exactly what is
    // missing rather than a generic "not implemented".
    match ForageBuildConfig::load() {
        Ok(cfg) => match cfg.require_base_platform() {
            Ok(platform) => eprintln!(
                "{} base platform {} is ready and packages would be written to {}; \
                 the sandboxed build-execution wiring is the next step, so no package \
                 was produced.",
                "note:".yellow().bold(),
                platform.display(),
                cfg.out_dir().display()
            ),
            Err(e) => eprintln!("{} {e}", "note:".yellow().bold()),
        },
        Err(e) => eprintln!("{} {e}", "error:".red().bold()),
    }
    exit(1);
}
