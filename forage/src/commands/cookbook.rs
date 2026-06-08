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
use colored::Colorize;
use serde::{Deserialize, Serialize};

/// One tracked cookbook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Cookbook {
    /// The tap name, a safe path component used as its clone directory.
    name: String,
    /// `git+<https url>` for a cloned cookbook, or a local directory path.
    source: String,
}

/// The tracked-cookbook registry, ordered by precedence (first = highest).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Registry {
    #[serde(default)]
    cookbook: Vec<Cookbook>,
}

impl Registry {
    /// Load the registry from `~/.config/arlen/cookbooks.toml`, or an empty one
    /// if the file is absent.
    fn load() -> Result<Self, String> {
        let path = registry_path();
        if !path.exists() {
            return Ok(Registry::default());
        }
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        toml::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))
    }

    /// Write the registry back atomically (temp file then rename).
    fn save(&self) -> Result<(), String> {
        let path = registry_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| format!("serialise registry: {e}"))?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).map_err(|e| format!("write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("rename into place: {e}"))
    }
}

/// `~/.config/arlen/cookbooks.toml`.
fn registry_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("arlen/cookbooks.toml")
}

/// `~/.local/share/arlen/forage/cookbooks/<name>` — where a git cookbook clones.
fn clone_dir(name: &str) -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("arlen/forage/cookbooks")
        .join(name)
}

/// Whether `name` is safe as a single path component: non-empty, no separators,
/// not a relative special, only `[A-Za-z0-9._-]`, and not `.`/`..`.
fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

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

    if let Some(url) = source.strip_prefix("git+") {
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
    } else {
        // A local cookbook directory: it must exist; it is referenced in place.
        if !Path::new(&source).is_dir() {
            eprintln!(
                "{} local cookbook '{source}' is not a directory (use git+<url> for a remote)",
                "error:".red().bold()
            );
            exit(1);
        }
    }

    registry.cookbook.push(Cookbook {
        name: name.clone(),
        source,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_validation_rejects_unsafe_components() {
        assert!(is_valid_name("personal"));
        assert!(is_valid_name("my-tap_1.0"));
        assert!(!is_valid_name(""));
        assert!(!is_valid_name("."));
        assert!(!is_valid_name(".."));
        assert!(!is_valid_name("a/b"));
        assert!(!is_valid_name("../escape"));
        assert!(!is_valid_name("has space"));
    }

    #[test]
    fn registry_round_trips_in_precedence_order() {
        let r = Registry {
            cookbook: vec![
                Cookbook {
                    name: "personal".into(),
                    source: "/home/me/tap".into(),
                },
                Cookbook {
                    name: "official".into(),
                    source: "git+https://x/o".into(),
                },
            ],
        };
        let text = toml::to_string_pretty(&r).unwrap();
        let back: Registry = toml::from_str(&text).unwrap();
        assert_eq!(back.cookbook, r.cookbook);
        assert_eq!(back.cookbook[0].name, "personal");
    }

    #[test]
    fn empty_registry_parses_from_absent_table() {
        let r: Registry = toml::from_str("").unwrap();
        assert!(r.cookbook.is_empty());
    }
}
