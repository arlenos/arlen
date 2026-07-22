//! The `forage install <app>` bridge auto-install flow (`foreign-app-bridges.md`
//! §4): when a foreign app is installed, every cookbook bridge tagged for it is
//! installed alongside, in the SAME transaction. This module owns the transactional
//! batch - install each prepared bridge's two halves and grant its delegated
//! namespace, and if ANY bridge in the batch fails, unwind the whole batch (remove
//! every placed file and revoke every namespace this batch granted) so a partial
//! `forage install` never leaves a half-wired bridge or a live grant with no bridge
//! behind it.
//!
//! The per-bridge mechanism (path-safe copy, namespace provisioning, single-bridge
//! rollback reporting) lives in `arlen-forage-bridge-install`; this is the multi-
//! bridge orchestration on top of it. Fetching + verifying each bridge recipe (the
//! cookbook-resolve + pinned-commit clone) and discovering the foreign-app token set
//! (e.g. `$VAULT` for Obsidian) are the caller's job - a [`PreparedBridge`] arrives
//! already fetched, parsed and namespace-resolved, so this core is pure and tested
//! without a network or a cookbook.
// Mechanism ahead of its consumer: the `forage install <app>` trigger hook that
// fetches the bridges and calls this is a following slice, like `bridges_in_cookbooks`.
#![allow(dead_code)]

use arlen_forage_bridge_install::{
    arlen_bridge_dir, bridge_namespace, deprovision_bridge_namespace, install_bridge,
    is_safe_relative_path, resolve_foreign_dest, BridgeInstallResult, InstallBridgeError,
    InstalledBridge, NamespaceError,
};
use arlen_forage_recipe::{Install, Recipe};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A bridge that has been fetched, verified and parsed, ready for the transactional
/// install: its recipe id (scopes the Arlen-side dir), its verified source checkout,
/// its `[install]` manifest, and its validated delegated namespace.
#[derive(Debug, Clone)]
pub struct PreparedBridge {
    /// The recipe id (e.g. `md.obsidian.bridge`); scopes the Arlen-side bridge dir.
    pub recipe_id: String,
    /// The verified recipe checkout the two halves are copied from.
    pub source_dir: PathBuf,
    /// The `[install]` manifest (Arlen-side files + the foreign-side plugin drop).
    pub install: Install,
    /// The validated delegated namespace (from the bridge's `entities.toml`).
    pub namespace: String,
}

/// A failure turning a fetched bridge recipe into a [`PreparedBridge`].
#[derive(Debug, thiserror::Error)]
pub enum PrepareError {
    /// The recipe has no `[bridge]` section (not a bridge recipe).
    #[error("recipe '{0}' is not a bridge (no [bridge] section)")]
    NotABridge(String),
    /// The recipe has no `[install]` manifest (a bridge requires one).
    #[error("bridge '{0}' has no [install] manifest")]
    NoInstall(String),
    /// No `entities.toml` in the arlen-side files (nowhere to read the namespace).
    #[error("bridge '{0}' declares no entities.toml in its arlen_side")]
    NoEntities(String),
    /// The arlen-side entities path is unsafe (absolute or `..`), so reading it
    /// could escape the recipe source; refused before any read.
    #[error("bridge '{recipe_id}': unsafe entities path: {path}")]
    UnsafeEntitiesPath {
        /// The bridge recipe id.
        recipe_id: String,
        /// The offending declared path.
        path: String,
    },
    /// Reading the entities.toml from the source checkout failed.
    #[error("bridge '{recipe_id}' entities.toml: {source}")]
    Io {
        /// The bridge recipe id.
        recipe_id: String,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// The declared namespace is missing, reserved, or malformed.
    #[error("bridge '{recipe_id}' namespace: {source}")]
    Namespace {
        /// The bridge recipe id.
        recipe_id: String,
        /// The namespace validation error.
        source: NamespaceError,
    },
}

/// Turn a fetched, verified bridge recipe checkout into a [`PreparedBridge`]: pull
/// the `[install]` manifest and read the delegated namespace from the arlen-side
/// `entities.toml` (the schema-registration file that carries the top-level
/// `namespace`). Pure over an already-fetched source dir + parsed recipe - the
/// network fetch and pinned-commit + hash verification are the caller's job, so this
/// is fully testable without a network.
///
/// The entities path is confined to the source tree (safe-relative, no `..`) before
/// it is read, so a recipe cannot point the namespace read at `/etc/passwd`; the
/// namespace itself is validated (non-reserved, well-formed) via [`bridge_namespace`].
pub fn bridge_from_source(source_dir: &Path, recipe: &Recipe) -> Result<PreparedBridge, PrepareError> {
    let recipe_id = recipe.recipe.id.clone();
    if recipe.bridge.is_none() {
        return Err(PrepareError::NotABridge(recipe_id));
    }
    let install = recipe
        .install
        .clone()
        .ok_or_else(|| PrepareError::NoInstall(recipe_id.clone()))?;

    // The namespace lives in the arlen-side entities.toml (schema registration).
    let entities_rel = install
        .arlen_side
        .iter()
        .find(|p| p.file_name().and_then(|n| n.to_str()) == Some("entities.toml"))
        .ok_or_else(|| PrepareError::NoEntities(recipe_id.clone()))?;
    if !is_safe_relative_path(entities_rel) {
        return Err(PrepareError::UnsafeEntitiesPath {
            recipe_id,
            path: entities_rel.display().to_string(),
        });
    }
    let contents = std::fs::read_to_string(source_dir.join(entities_rel))
        .map_err(|source| PrepareError::Io { recipe_id: recipe_id.clone(), source })?;
    let namespace = bridge_namespace(&contents)
        .map_err(|source| PrepareError::Namespace { recipe_id: recipe_id.clone(), source })?;

    Ok(PreparedBridge {
        recipe_id,
        source_dir: source_dir.to_path_buf(),
        install,
        namespace,
    })
}

/// A bridge batch-install failure. Every variant means the whole batch was unwound
/// (files removed, this-batch namespace grants revoked) before it was returned, so
/// the caller sees a clean state.
#[derive(Debug, thiserror::Error)]
pub enum BridgeFlowError {
    /// The `foreign_side.into` template did not resolve to a safe destination.
    #[error("bridge '{recipe_id}': {source}")]
    Template {
        /// The offending bridge.
        recipe_id: String,
        /// The template error.
        source: arlen_forage_bridge_install::TemplateError,
    },
    /// The Arlen-side bridge dir could not be resolved (no `XDG_DATA_HOME`/`HOME`).
    #[error("bridge '{recipe_id}': cannot resolve the Arlen bridge dir (set XDG_DATA_HOME or HOME)")]
    NoBridgeDir {
        /// The offending bridge.
        recipe_id: String,
    },
    /// Installing a bridge failed; the batch was rolled back.
    #[error("bridge '{recipe_id}': {source}")]
    Install {
        /// The offending bridge.
        recipe_id: String,
        /// The single-bridge install error.
        source: InstallBridgeError,
    },
}

/// Install a batch of prepared bridges transactionally. Each bridge's foreign-side
/// destination is resolved from `tokens` (e.g. `VAULT` -> the user's vault), its
/// two halves are copied, and its namespace is granted. On the first failure the
/// whole batch is unwound - every already-placed file is removed and every namespace
/// THIS batch granted is revoked (a namespace a prior install already held, reported
/// `namespace_granted = false`, is left) - so a failed `forage install` is atomic.
///
/// Returns the per-bridge results on full success. The caller owns the ONE install-
/// time consent upstream and the fetch/parse that produced each [`PreparedBridge`].
pub fn install_prepared_bridges(
    prepared: &[PreparedBridge],
    tokens: &HashMap<String, PathBuf>,
) -> Result<Vec<BridgeInstallResult>, BridgeFlowError> {
    let mut done: Vec<(String, BridgeInstallResult)> = Vec::new();

    for pb in prepared {
        let foreign_dest = match resolve_foreign_dest(&pb.install.foreign_side.into, tokens) {
            Ok(d) => d,
            Err(source) => {
                unwind(&done);
                return Err(BridgeFlowError::Template { recipe_id: pb.recipe_id.clone(), source });
            }
        };
        let Some(arlen_dir) = arlen_bridge_dir(&pb.recipe_id) else {
            unwind(&done);
            return Err(BridgeFlowError::NoBridgeDir { recipe_id: pb.recipe_id.clone() });
        };

        match install_bridge(&pb.source_dir, &pb.install, &arlen_dir, &foreign_dest, &pb.namespace) {
            Ok(res) => done.push((pb.namespace.clone(), res)),
            Err(source) => {
                // Roll back this bridge's own partial write first, then the batch.
                rollback_failed_bridge(&source);
                unwind(&done);
                return Err(BridgeFlowError::Install { recipe_id: pb.recipe_id.clone(), source });
            }
        }
    }

    Ok(done.into_iter().map(|(_, res)| res).collect())
}

/// Unwind the successfully-installed bridges of a failed batch: remove every placed
/// file and revoke every namespace THIS batch granted (`namespace_granted == true`).
/// Best-effort - an already-gone file or an unwritable profile does not abort the
/// unwind (the batch has already failed; this only cleans up).
fn unwind(done: &[(String, BridgeInstallResult)]) {
    for (namespace, res) in done {
        arlen_forage_bridge_install::rollback_bridge(&res.installed);
        if res.namespace_granted {
            let _ = deprovision_bridge_namespace(namespace);
        }
    }
}

/// Roll back the partial write of the bridge that failed. Its error carries whatever
/// files were placed before it failed (a copy I/O error, or a copy that succeeded
/// then a profile provisioning failure); a pre-flight failure placed nothing.
fn rollback_failed_bridge(err: &InstallBridgeError) {
    let placed: Option<&InstalledBridge> = match err {
        InstallBridgeError::Copy(
            arlen_forage_bridge_install::BridgeInstallError::Io { wrote, .. },
        ) => Some(wrote),
        InstallBridgeError::Provision { installed, .. } => Some(installed),
        // A pre-flight (UnsafePath/BadSource) failure placed nothing.
        InstallBridgeError::Copy(_) => None,
    };
    if let Some(p) = placed {
        arlen_forage_bridge_install::rollback_bridge(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_forage_recipe::{ForeignSide, Install};
    use std::fs;
    use std::sync::Mutex;

    /// Serializes the tests that set `ARLEN_PERMISSIONS_DIR`/`XDG_DATA_HOME` (process
    /// env is global).
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn seed(dir: &std::path::Path, rel: &str, contents: &str) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, contents).unwrap();
    }

    /// A prepared bridge whose source is a fresh temp checkout with the named files.
    fn prepare(
        recipe_id: &str,
        namespace: &str,
        arlen_files: &[&str],
        foreign_files: &[&str],
    ) -> (tempfile::TempDir, PreparedBridge) {
        let src = tempfile::tempdir().unwrap();
        for f in arlen_files.iter().chain(foreign_files.iter()) {
            seed(src.path(), f, "x");
        }
        let install = Install {
            arlen_side: arlen_files.iter().map(PathBuf::from).collect(),
            foreign_side: ForeignSide {
                into: "$VAULT/.obsidian/plugins/b/".to_string(),
                files: foreign_files.iter().map(PathBuf::from).collect(),
            },
        };
        let pb = PreparedBridge {
            recipe_id: recipe_id.to_string(),
            source_dir: src.path().to_path_buf(),
            install,
            namespace: namespace.to_string(),
        };
        (src, pb)
    }

    fn vault_tokens(vault: &std::path::Path) -> HashMap<String, PathBuf> {
        let mut t = HashMap::new();
        t.insert("VAULT".to_string(), vault.to_path_buf());
        t
    }

    /// A minimal bridge recipe (deserialized, NOT validated - we exercise
    /// `bridge_from_source`, not recipe validation, so `toml::from_str` suffices).
    fn bridge_recipe(id: &str, arlen_side: &[&str]) -> Recipe {
        let files = arlen_side
            .iter()
            .map(|f| format!("\"{f}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let toml = format!(
            "[recipe]\nid = \"{id}\"\nname = \"B\"\nmaintainer = \"k1\"\n\
             [bridge]\nforeign_app = \"obsidian\"\n\
             [install]\narlen_side = [{files}]\n\
             [install.foreign_side]\ninto = \"$VAULT/.obsidian/plugins/b/\"\nfiles = [\"main.js\"]\n"
        );
        toml::from_str(&toml).unwrap()
    }

    #[test]
    fn bridge_from_source_extracts_the_install_and_namespace() {
        let src = tempfile::tempdir().unwrap();
        seed(src.path(), "entities.toml", "namespace = \"md.obsidian\"\n");
        seed(src.path(), "bridge.toml", "x");
        let recipe = bridge_recipe("md.obsidian.bridge", &["entities.toml", "bridge.toml"]);

        let pb = bridge_from_source(src.path(), &recipe).unwrap();
        assert_eq!(pb.recipe_id, "md.obsidian.bridge");
        assert_eq!(pb.namespace, "md.obsidian");
        assert_eq!(pb.install.arlen_side.len(), 2);
        assert_eq!(pb.source_dir, src.path());
    }

    #[test]
    fn a_non_bridge_recipe_is_refused() {
        // A recipe with no [bridge] section is not a bridge.
        let recipe: Recipe =
            toml::from_str("[recipe]\nid = \"x\"\nname = \"X\"\nmaintainer = \"k\"\n").unwrap();
        let src = tempfile::tempdir().unwrap();
        assert!(matches!(
            bridge_from_source(src.path(), &recipe),
            Err(PrepareError::NotABridge(_))
        ));
    }

    #[test]
    fn a_bridge_with_no_entities_toml_is_refused() {
        let src = tempfile::tempdir().unwrap();
        seed(src.path(), "bridge.toml", "x");
        let recipe = bridge_recipe("md.obsidian.bridge", &["bridge.toml"]);
        assert!(matches!(
            bridge_from_source(src.path(), &recipe),
            Err(PrepareError::NoEntities(_))
        ));
    }

    #[test]
    fn an_unsafe_entities_path_is_refused_before_reading() {
        let src = tempfile::tempdir().unwrap();
        // A recipe naming a traversing entities path must be refused before the read,
        // so it cannot point the namespace read outside its own source tree.
        let recipe = bridge_recipe("md.obsidian.bridge", &["../secret/entities.toml"]);
        assert!(matches!(
            bridge_from_source(src.path(), &recipe),
            Err(PrepareError::UnsafeEntitiesPath { .. })
        ));
    }

    #[test]
    fn a_reserved_namespace_is_refused() {
        let src = tempfile::tempdir().unwrap();
        seed(src.path(), "entities.toml", "namespace = \"system.core\"\n");
        let recipe = bridge_recipe("evil.bridge", &["entities.toml"]);
        assert!(matches!(
            bridge_from_source(src.path(), &recipe),
            Err(PrepareError::Namespace { .. })
        ));
    }

    #[test]
    fn a_two_bridge_batch_installs_both_and_grants_both_namespaces() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let perms = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let vault = tempfile::tempdir().unwrap();
        std::env::set_var("ARLEN_PERMISSIONS_DIR", perms.path());
        std::env::set_var("XDG_DATA_HOME", data.path());

        let (_a, a) = prepare("md.obsidian.bridge", "md.obsidian", &["entities.toml"], &["main.js"]);
        let (_b, b) = prepare("com.zotero.bridge", "com.zotero", &["entities.toml"], &["main.js"]);

        let results = install_prepared_bridges(&[a, b], &vault_tokens(vault.path())).unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.namespace_granted));
        // Both foreign sides landed under the vault.
        assert!(vault.path().join(".obsidian/plugins/b/main.js").exists());
        // The shared profile accumulated both namespaces.
        let profile: arlen_permissions::PermissionProfile = toml::from_str(
            &fs::read_to_string(perms.path().join("bridge-ingest.toml")).unwrap(),
        )
        .unwrap();
        assert_eq!(profile.graph.delegated_namespaces, vec!["md.obsidian", "com.zotero"]);

        std::env::remove_var("ARLEN_PERMISSIONS_DIR");
        std::env::remove_var("XDG_DATA_HOME");
    }

    #[test]
    fn a_failing_second_bridge_unwinds_the_whole_batch() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let perms = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let vault = tempfile::tempdir().unwrap();
        std::env::set_var("ARLEN_PERMISSIONS_DIR", perms.path());
        std::env::set_var("XDG_DATA_HOME", data.path());

        // First bridge is valid; the second names a missing source file, so it fails
        // pre-flight after the first already installed.
        let (_a, a) = prepare("md.obsidian.bridge", "md.obsidian", &["entities.toml"], &["main.js"]);
        let src_b = tempfile::tempdir().unwrap();
        seed(src_b.path(), "entities.toml", "x"); // main.js is deliberately absent
        let bad = PreparedBridge {
            recipe_id: "com.zotero.bridge".to_string(),
            source_dir: src_b.path().to_path_buf(),
            install: Install {
                arlen_side: vec![PathBuf::from("entities.toml")],
                foreign_side: ForeignSide {
                    into: "$VAULT/.zotero/b/".to_string(),
                    files: vec![PathBuf::from("main.js")],
                },
            },
            namespace: "com.zotero".to_string(),
        };

        let err = install_prepared_bridges(&[a, bad], &vault_tokens(vault.path())).unwrap_err();
        assert!(matches!(err, BridgeFlowError::Install { .. }));

        // The first bridge's files are gone (batch unwound) and its namespace grant
        // was revoked, leaving no bridge-ingest profile grant behind.
        assert!(!data.path().join("arlen/bridges/md.obsidian.bridge/entities.toml").exists());
        assert!(!vault.path().join(".obsidian/plugins/b/main.js").exists());
        let profile_path = perms.path().join("bridge-ingest.toml");
        if profile_path.exists() {
            let profile: arlen_permissions::PermissionProfile =
                toml::from_str(&fs::read_to_string(&profile_path).unwrap()).unwrap();
            assert!(
                profile.graph.delegated_namespaces.is_empty(),
                "the rolled-back batch must leave no delegated namespace"
            );
        }

        std::env::remove_var("ARLEN_PERMISSIONS_DIR");
        std::env::remove_var("XDG_DATA_HOME");
    }

    #[test]
    fn an_unresolvable_template_fails_before_installing() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let perms = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        std::env::set_var("ARLEN_PERMISSIONS_DIR", perms.path());
        std::env::set_var("XDG_DATA_HOME", data.path());

        let (_a, a) = prepare("md.obsidian.bridge", "md.obsidian", &["entities.toml"], &["main.js"]);
        // No VAULT token -> the template cannot resolve -> fail before any write.
        let err = install_prepared_bridges(&[a], &HashMap::new()).unwrap_err();
        assert!(matches!(err, BridgeFlowError::Template { .. }));
        assert!(!data.path().join("arlen/bridges/md.obsidian.bridge/entities.toml").exists());

        std::env::remove_var("ARLEN_PERMISSIONS_DIR");
        std::env::remove_var("XDG_DATA_HOME");
    }
}
