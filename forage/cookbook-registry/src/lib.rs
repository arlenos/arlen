//! The tracked-cookbook registry: which cookbooks this machine trusts, where each
//! one lives on disk, and how a recipe name resolves across them.
//!
//! Lifted out of the `forage` binary because it was private there while being the
//! only place that knows a recipe's PROVENANCE. The store composes app cards from
//! recipe text alone and so can name no publisher and show no cookbook tier - not
//! for want of a field, but because the fact lived in a binary nothing can depend
//! on. Anything that needs to ask "which cookbook vouches for this, and is its
//! root still the one the user pinned" belongs here.
//!
//! Resolution is fail-closed throughout: an unsigned cookbook never resolves, and
//! a pinned cookbook whose on-disk root no longer matches its pin is a hard error
//! rather than a fall-through to the next cookbook - a trusted root changing
//! underneath the user is exactly the event the pin exists to catch.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One tracked cookbook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cookbook {
    /// The tap name, a safe path component used as its clone directory.
    pub name: String,
    /// `git+<https url>` for a cloned cookbook, or a local directory path.
    pub source: String,
    /// The sha256 (lowercase hex) of the cookbook's TUF `metadata/root.json`,
    /// pinned on `add` (trust on first use). `None` for an unsigned cookbook,
    /// which resolution refuses to install from. Pinning the hash rather than a
    /// path means a later tampering of the on-disk root is caught at resolve
    /// time, when the file is re-read and checked against this pin.
    #[serde(default)]
    pub pinned_root_sha256: Option<String>,
}

/// The tracked-cookbook registry, ordered by precedence (first = highest).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default)]
    pub cookbook: Vec<Cookbook>,
}

impl Registry {
    /// Load the registry from `~/.config/arlen/cookbooks.toml`, or an empty one
    /// if the file is absent.
    pub fn load() -> Result<Self, String> {
        let path = registry_path();
        if !path.exists() {
            return Ok(Registry::default());
        }
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        toml::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))
    }

    /// Write the registry back atomically (temp file then rename).
    pub fn save(&self) -> Result<(), String> {
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
pub fn registry_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("arlen/cookbooks.toml")
}

/// `~/.local/share/arlen/forage/cookbooks/<name>` — where a git cookbook clones.
pub fn clone_dir(name: &str) -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("arlen/forage/cookbooks")
        .join(name)
}

/// The directory holding a cookbook's signed TUF metadata: `<root>/metadata`,
/// where the root is the tracked clone for a `git+` cookbook or the source
/// directory for a local one.
pub fn cookbook_metadata_dir(c: &Cookbook) -> PathBuf {
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
pub async fn resolve_against(
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

/// Enumerate every bridge tagged for `foreign_app` across the tracked cookbooks
/// (foreign-app-bridges.md §4: `forage install <app>` finds the bridges to install
/// when the app is installed). Loads the registry and collects from every signed,
/// pinned cookbook. The consumer is the `forage install <app>` bridge-install flow
/// (its consent gate is the remaining piece), so this is wired there, not exposed
/// as its own command (the design auto-installs, it does not add a query verb).
pub async fn bridges_in_cookbooks(
    foreign_app: &str,
) -> Result<Vec<arlen_cookbook_resolve::ResolvedRecipe>, String> {
    let registry = Registry::load()?;
    bridges_against(&registry.cookbook, foreign_app).await
}

/// The bridge-discovery core over an explicit cookbook list (testable without the
/// global registry path). Unlike [`resolve_against`] (first-match single recipe),
/// this COLLECTS: a foreign app may be bridged by more than one cookbook. Layered
/// by precedence, deduped by recipe name (the first cookbook to carry a name wins).
/// A pinned-root mismatch fails closed (tamper); an unsigned or root-less cookbook
/// is skipped; a malformed matching target propagates the error (fail-closed).
pub async fn bridges_against(
    cookbooks: &[Cookbook],
    foreign_app: &str,
) -> Result<Vec<arlen_cookbook_resolve::ResolvedRecipe>, String> {
    let mut out: Vec<arlen_cookbook_resolve::ResolvedRecipe> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for c in cookbooks {
        // Unsigned cookbooks never resolve (fail-closed), like resolve_against.
        let Some(pin) = &c.pinned_root_sha256 else {
            continue;
        };
        let metadata_dir = cookbook_metadata_dir(c);
        let root_bytes = match std::fs::read(metadata_dir.join("root.json")) {
            Ok(b) => b,
            Err(_) => continue, // incomplete/removed clone; skip
        };
        if sha256_hex(&root_bytes) != *pin {
            return Err(format!(
                "cookbook '{}' root.json no longer matches its pinned hash; refusing (re-add it if the change is expected)",
                c.name
            ));
        }
        let bridges =
            arlen_cookbook_resolve::enumerate_bridges_for(&root_bytes, &metadata_dir, foreign_app)
                .await
                .map_err(|e| format!("cookbook '{}': {e}", c.name))?;
        for b in bridges {
            if seen.insert(b.name.clone()) {
                out.push(b);
            }
        }
    }
    Ok(out)
}

/// Compute the trust pin for a cookbook: the sha256 (lowercase hex) of its
/// `metadata/root.json`. Returns `None` if the cookbook has no such root (it is
/// unsigned), reading via `symlink_metadata` discipline left to the caller's
/// tracked, owned clone directory.
pub fn root_pin(metadata_dir: &Path) -> Option<String> {
    let root = metadata_dir.join("root.json");
    let bytes = std::fs::read(&root).ok()?;
    Some(sha256_hex(&bytes))
}

/// Lowercase-hex sha256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether `name` is safe as a single path component: non-empty, no separators,
/// not a relative special, only `[A-Za-z0-9._-]`, and not `.`/`..`.
pub fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
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

    /// Like [`signed_cookbook_fixture`] but the recipe carries a `foreign_app` tag,
    /// so it is a BRIDGE `enumerate_bridges_for` returns.
    async fn signed_bridge_fixture(
        cookbook_dir: &Path,
        recipe_name: &str,
        foreign_app: &str,
    ) -> Cookbook {
        use arlen_cookbook_sign::{generate_signing_key, sign_cookbook, Expiries, SignParams};
        let md = cookbook_dir.join("metadata");
        std::fs::create_dir_all(&md).unwrap();
        let key = cookbook_dir.join("key.der");
        generate_signing_key(&key).unwrap();
        let recipe_bytes = format!("name = \"{recipe_name}\"\n").into_bytes();
        let hash = sha256_hex(&recipe_bytes);
        let toml = format!(
            "[[recipe]]\nname = \"{recipe_name}\"\ngit_url = \"github.com/o/r\"\ncommit = \"deadbeefdeadbeefdeadbeefdeadbeefdeadbeef\"\nrecipe_hash = \"{hash}\"\nforeign_app = \"{foreign_app}\"\n"
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
    async fn bridges_against_finds_the_matching_foreign_app() {
        let dir = tempfile::tempdir().unwrap();
        let cb = signed_bridge_fixture(&dir.path().join("taps"), "md.obsidian.bridge", "obsidian").await;
        let found = bridges_against(std::slice::from_ref(&cb), "obsidian").await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "md.obsidian.bridge");
        // A different app has no bridge here.
        assert!(bridges_against(&[cb], "vscode").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn bridges_against_dedups_by_recipe_name_across_cookbooks() {
        let dir = tempfile::tempdir().unwrap();
        let a = signed_bridge_fixture(&dir.path().join("a"), "md.obsidian.bridge", "obsidian").await;
        let b = signed_bridge_fixture(&dir.path().join("b"), "md.obsidian.bridge", "obsidian").await;
        // Same recipe name in two cookbooks -> one result (first-precedence wins).
        let found = bridges_against(&[a, b], "obsidian").await.unwrap();
        assert_eq!(found.len(), 1);
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
