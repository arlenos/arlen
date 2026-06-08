//! Challenge subcommand.
//!
//! `forage challenge` rebuilds a package independently and compares the output
//! hash, feeding the pantry's reproducibility consensus. The build pipeline and
//! trust mechanics land in forage-recipes.md R1/R4; this is a surface stub.

use std::process::exit;

use colored::Colorize;

/// Challenge a build's reproducibility for the given app or recipe id.
pub fn challenge(target: &str) {
    eprintln!(
        "{} reproducibility challenges are not yet implemented \
         (forage-recipes.md R4) [{target}]",
        "note:".yellow().bold()
    );
    exit(1);
}
