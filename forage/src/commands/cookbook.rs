//! Cookbook subcommands: add, remove, list.
//!
//! A cookbook is a git-based (or local) recipe index, exactly Homebrew's tap
//! model (forage-recipes.md section 7). `add` clones a `git+URL` cookbook (or
//! registers a local directory) and tracks it in `~/.config/arlen/cookbooks.toml`;
//! `list` shows the tracked cookbooks in precedence order; `remove` drops one
//! and deletes its clone. Cookbooks are layered with the user's precedence
//! (personal first); the layered *resolution* that uses that order, and the
//! TUF-style index trust, land in forage-recipes.md R3 and are not here yet.
//! This is the tap-management surface those build on.

use std::path::{Path, PathBuf};
use std::process::exit;

use arlen_forage_fetch::{clone_recipe_repo, DEFAULT_RECIPE_REPO_BYTES};
use arlen_cookbook_registry::{
    clone_dir, cookbook_metadata_dir, is_valid_name, registry_path, root_pin, Cookbook, Registry,
};
pub use arlen_cookbook_registry::{bridges_in_cookbooks, resolve_in_cookbooks};
use colored::Colorize;

/// Add a cookbook by name and source (`git+<url>` or a local directory).
pub async fn add(name: String, source: String) {
    if !is_valid_name(&name) {
        eprintln!(
            "{} cookbook name '{name}' must be non-empty and only contain letters, digits, '.', '_' or '-'",
            "error:".red().bold()
        );
        exit(1);
    }
    let mut registry = match Registry::load() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{} {e}", "error:".red().bold());
            exit(1);
        }
    };
    if registry.cookbook.iter().any(|c| c.name == name) {
        eprintln!("{} a cookbook named '{name}' is already tracked", "error:".red().bold());
        exit(1);
    }

    // The cookbook's signed metadata lives at `<root>/metadata/` (section 7a).
    let metadata_dir = if let Some(url) = source.strip_prefix("git+") {
        // Clone the cookbook repo's working tree into its tracked location.
        let dest = clone_dir(&name);
        if let Some(parent) = dest.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!("{} preparing clone dir: {e}", "error:".red().bold());
                exit(1);
            }
        }
        if let Err(e) = clone_recipe_repo(url, None, &dest, DEFAULT_RECIPE_REPO_BYTES).await {
            eprintln!("{} cloning {url}: {e}", "error:".red().bold());
            let _ = std::fs::remove_dir_all(&dest);
            exit(1);
        }
        dest.join("metadata")
    } else {
        // A local cookbook directory: it must exist; it is referenced in place.
        if !Path::new(&source).is_dir() {
            eprintln!(
                "{} local cookbook '{source}' is not a directory (use git+<url> for a remote)",
                "error:".red().bold()
            );
            exit(1);
        }
        Path::new(&source).join("metadata")
    };

    // Pin the root on first use. A cookbook with no signed metadata is still
    // tracked (so it lists and supports future in-repo discovery), but it is
    // recorded unsigned and resolution refuses to install from it.
    let pinned_root_sha256 = match root_pin(&metadata_dir) {
        Some(hash) => {
            println!("{} root {}", "pinned".green().bold(), &hash[..16.min(hash.len())]);
            Some(hash)
        }
        None => {
            eprintln!(
                "{} cookbook '{name}' has no signed metadata/root.json; it is tracked but not install-resolvable until signed",
                "warning:".yellow().bold()
            );
            None
        }
    };

    registry.cookbook.push(Cookbook {
        name: name.clone(),
        source,
        pinned_root_sha256,
    });
    if let Err(e) = registry.save() {
        eprintln!("{} {e}", "error:".red().bold());
        exit(1);
    }
    println!("{} cookbook '{name}'", "added".green().bold());
}

/// Remove a tracked cookbook by name and delete its clone, if any.
pub fn remove(name: &str) {
    let mut registry = match Registry::load() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{} {e}", "error:".red().bold());
            exit(1);
        }
    };
    let before = registry.cookbook.len();
    registry.cookbook.retain(|c| c.name != name);
    if registry.cookbook.len() == before {
        eprintln!("{} no cookbook named '{name}' is tracked", "error:".red().bold());
        exit(1);
    }
    if let Err(e) = registry.save() {
        eprintln!("{} {e}", "error:".red().bold());
        exit(1);
    }
    // Remove the clone directory (a local cookbook has none under our store).
    // Guard the destructive delete on a valid name even though `add` only ever
    // stores validated names: a hand-edited registry must not redirect the
    // recursive remove outside the cookbook store.
    if is_valid_name(name) {
        let dir = clone_dir(name);
        if dir.exists() {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
    println!("{} cookbook '{name}'", "removed".green().bold());
}

/// List tracked cookbooks in precedence order (first = highest).
pub fn list() {
    let registry = match Registry::load() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{} {e}", "error:".red().bold());
            exit(1);
        }
    };
    if registry.cookbook.is_empty() {
        println!(
            "no cookbooks tracked ({})",
            "forage cookbook add <name> git+<url>".dimmed()
        );
        return;
    }
    for (i, c) in registry.cookbook.iter().enumerate() {
        println!("{}. {} {}", i + 1, c.name.bold(), c.source.dimmed());
    }
}

/// Update cookbook indexes from their remotes (R3, layered resolution).
pub fn update() {
    eprintln!(
        "{} cookbook update lands with layered resolution (forage-recipes.md R3)",
        "note:".yellow().bold()
    );
    exit(1);
}
