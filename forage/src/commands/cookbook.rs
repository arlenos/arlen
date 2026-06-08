//! Cookbook subcommands: add, remove, list, update.
//!
//! Cookbooks are git-based recipe indexes (taps). Resolution, versioning and
//! TUF-style trust land in forage-recipes.md R3; these are surface stubs.

use std::process::exit;

use colored::Colorize;

const PENDING: &str = "cookbook management is not yet implemented (forage-recipes.md R3)";

/// Add a cookbook by name and git URL.
pub fn add(name: &str, url: &str) {
    pending(&format!("add '{name}' -> {url}"));
}

/// Remove a cookbook by name.
pub fn remove(name: &str) {
    pending(&format!("remove '{name}'"));
}

/// List configured cookbooks.
pub fn list() {
    pending("list");
}

/// Update cookbook indexes from their remotes.
pub fn update() {
    pending("update");
}

fn pending(action: &str) {
    eprintln!("{} {PENDING} [{action}]", "note:".yellow().bold());
    exit(1);
}
