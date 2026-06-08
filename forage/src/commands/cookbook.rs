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
    /// The sha256 (lowercase hex) of the cookbook's TUF `metadata/root.json`,
    /// pinned on `add` (trust on first use). `None` for an unsigned cookbook,
    /// which resolution refuses to install from. Pinning the hash rather than a
    /// path means a later tampering of the on-disk root is caught at resolve
    /// time, when the file is re-read and checked against this pin.
    #[serde(default)]
    pinned_root_sha256: Option<String>,
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

/// The directory holding a cookbook's signed TUF metadata: `<root>/metadata`,
/// where the root is the tracked clone for a `git+` cookbook or the source
/// directory for a local one.
fn cookbook_metadata_dir(c: &Cookbook) -> PathBuf {
    let root = if c.source.starts_with("git+") {
        clone_dir(&c.name)
    } else {
        PathBuf::from(&c.source)
    };
    root.join("metadata")
}

/// Resolve a recipe name across the tracked cookbooks, verifying each against its
/// pinned root, and return the first verified match's authenticated pointer.
///
/// Layered precedence (first tracked = highest) with first-match-wins. A cookbook
/// is consulted only if it is pinned (unsigned cookbooks never resolve, the
/// fail-closed model). If a pinned cookbook's on-disk `root.json` no longer
/// hashes to its pin, that is a hard error (tampering or an un-pinned root
/// change on a cookbook the user explicitly trusts), not a silent fall-through
/// to a lower-precedence cookbook.
pub async fn resolve_in_cookbooks(
    recipe_name: &str,
) -> Result<arlen_cookbook_resolve::ResolvedRecipe, String> {
    let registry = Registry::load()?;
    resolve_against(&registry.cookbook, recipe_name).await
}

/// The layered-resolution core over an explicit cookbook list (testable without
/// the global registry path).
async fn resolve_against(
    cookbooks: &[Cookbook],
    recipe_name: &str,
) -> Result<arlen_cookbook_resolve::ResolvedRecipe, String> {
    if cookbooks.is_empty() {
        return Err("no cookbooks are tracked (forage cookbook add <name> git+<url>)".into());
    }
    let mut last_err: Option<String> = None;
    let mut considered = false;
    for c in cookbooks {
        // Unsigned cookbooks never resolve (fail-closed).
        let Some(pin) = &c.pinned_root_sha256 else {
            continue;
        };
        let metadata_dir = cookbook_metadata_dir(c);
        let root_path = metadata_dir.join("root.json");
        let root_bytes = match std::fs::read(&root_path) {
            Ok(b) => b,
            // The clone is missing its root (incomplete or removed); skip it
            // rather than fail the whole resolution.
            Err(_) => continue,
        };
        if sha256_hex(&root_bytes) != *pin {
            return Err(format!(
                "cookbook '{}' root.json no longer matches its pinned hash; refusing (re-add it if the change is expected)",
                c.name
            ));
        }
        considered = true;
        match arlen_cookbook_resolve::resolve(&root_bytes, &metadata_dir, recipe_name).await {
            Ok(resolved) => return Ok(resolved),
            // Verified, but this cookbook does not index the recipe: try the next.
            Err(arlen_cookbook_resolve::ResolveError::NotFound(_)) => continue,
            Err(e) => last_err = Some(format!("cookbook '{}': {e}", c.name)),
        }
    }
    if let Some(e) = last_err {
        return Err(e);
    }
    if !considered {
        return Err(
            "no tracked cookbook is signed and pinned; nothing to resolve from".into(),
        );
    }
    Err(format!("no tracked cookbook provides '{recipe_name}'"))
}

/// Compute the trust pin for a cookbook: the sha256 (lowercase hex) of its
/// `metadata/root.json`. Returns `None` if the cookbook has no such root (it is
/// unsigned), reading via `symlink_metadata` discipline left to the caller's
/// tracked, owned clone directory.
fn root_pin(metadata_dir: &Path) -> Option<String> {
    let root = metadata_dir.join("root.json");
    let bytes = std::fs::read(&root).ok()?;
    Some(sha256_hex(&bytes))
}

/// Lowercase-hex sha256 of `bytes`.
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
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
                    pinned_root_sha256: Some("a".repeat(64)),
                },
                Cookbook {
                    name: "official".into(),
                    source: "git+https://x/o".into(),
                    pinned_root_sha256: None,
                },
            ],
        };
        let text = toml::to_string_pretty(&r).unwrap();
        let back: Registry = toml::from_str(&text).unwrap();
        assert_eq!(back.cookbook, r.cookbook);
        assert_eq!(back.cookbook[0].name, "personal");
        assert_eq!(back.cookbook[0].pinned_root_sha256.as_deref(), Some(&"a".repeat(64)[..]));
    }

    #[test]
    fn root_pin_hashes_present_root_and_is_none_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let md = dir.path().join("metadata");
        std::fs::create_dir_all(&md).unwrap();
        // No root.json yet: unsigned.
        assert!(root_pin(&md).is_none());
        // With a root.json: the pin is its sha256.
        std::fs::write(md.join("root.json"), b"root-bytes").unwrap();
        assert_eq!(root_pin(&md), Some(sha256_hex(b"root-bytes")));
    }

    #[test]
    fn an_old_registry_without_the_pin_field_still_parses() {
        // Registries written before pinning existed have no pinned_root_sha256.
        let back: Registry =
            toml::from_str("[[cookbook]]\nname = \"x\"\nsource = \"/t\"\n").unwrap();
        assert_eq!(back.cookbook.len(), 1);
        assert!(back.cookbook[0].pinned_root_sha256.is_none());
    }

    /// Sign a one-recipe cookbook into `<cookbook_dir>/metadata` and return a
    /// tracked `Cookbook` entry for it (source = the local dir, root pinned).
    async fn signed_cookbook_fixture(cookbook_dir: &Path, recipe_name: &str) -> Cookbook {
        use arlen_cookbook_sign::{generate_signing_key, sign_cookbook, Expiries, SignParams};
        let md = cookbook_dir.join("metadata");
        std::fs::create_dir_all(&md).unwrap();
        let key = cookbook_dir.join("key.der");
        generate_signing_key(&key).unwrap();
        let recipe_bytes = format!("name = \"{recipe_name}\"\n").into_bytes();
        let hash = sha256_hex(&recipe_bytes);
        let toml = format!(
            "[[recipe]]\nname = \"{recipe_name}\"\ngit_url = \"github.com/o/r\"\ncommit = \"deadbeefdeadbeefdeadbeefdeadbeefdeadbeef\"\nrecipe_hash = \"{hash}\"\n"
        );
        let manifest = arlen_cookbook_index::parse(&toml).unwrap();
        let mut recipes = std::collections::HashMap::new();
        recipes.insert(recipe_name.to_string(), recipe_bytes);
        sign_cookbook(SignParams {
            manifest: &manifest,
            recipes: &recipes,
            key_path: &key,
            out_dir: &md,
            expiries: Expiries::defaults_from(chrono::Utc::now()),
        })
        .await
        .unwrap();
        let pin = root_pin(&md).unwrap();
        Cookbook {
            name: cookbook_dir.file_name().unwrap().to_string_lossy().into_owned(),
            source: cookbook_dir.to_string_lossy().into_owned(),
            pinned_root_sha256: Some(pin),
        }
    }

    #[tokio::test]
    async fn resolves_a_recipe_from_a_pinned_cookbook() {
        let dir = tempfile::tempdir().unwrap();
        let cb = signed_cookbook_fixture(&dir.path().join("personal"), "com.example.Tool").await;
        let resolved = resolve_against(&[cb], "com.example.Tool").await.unwrap();
        assert_eq!(resolved.git_url, "github.com/o/r");
    }

    #[tokio::test]
    async fn a_tampered_pinned_root_is_a_hard_error() {
        let dir = tempfile::tempdir().unwrap();
        let cbdir = dir.path().join("personal");
        let cb = signed_cookbook_fixture(&cbdir, "com.example.Tool").await;
        // Tamper the on-disk root after pinning: the stored pin no longer matches.
        std::fs::write(cbdir.join("metadata/root.json"), b"tampered").unwrap();
        let err = resolve_against(&[cb], "com.example.Tool").await.unwrap_err();
        assert!(err.contains("no longer matches its pinned hash"), "{err}");
    }

    #[tokio::test]
    async fn an_unsigned_cookbook_resolves_nothing() {
        let cb = Cookbook {
            name: "bare".into(),
            source: "/nonexistent".into(),
            pinned_root_sha256: None,
        };
        let err = resolve_against(&[cb], "com.example.Tool").await.unwrap_err();
        assert!(err.contains("signed and pinned"), "{err}");
    }

    #[tokio::test]
    async fn an_unknown_recipe_reports_no_provider() {
        let dir = tempfile::tempdir().unwrap();
        let cb = signed_cookbook_fixture(&dir.path().join("personal"), "com.example.Tool").await;
        let err = resolve_against(&[cb], "com.example.Other").await.unwrap_err();
        assert!(err.contains("no tracked cookbook provides"), "{err}");
    }

    #[test]
    fn empty_registry_parses_from_absent_table() {
        let r: Registry = toml::from_str("").unwrap();
        assert!(r.cookbook.is_empty());
    }
}
