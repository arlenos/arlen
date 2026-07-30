//! Reading the three sources into one inventory.
//!
//! Everything here is disk-only: enrolled profiles, module manifests, bridge
//! configs. Nothing asks a running daemon, which keeps this testable against a
//! temp tree and means the surface still answers when a daemon is down - a
//! management view that goes blank exactly when something is broken is the one
//! that fails when it is needed.
//!
//! The consequence is that [`Health`] is mostly `Unknown` from here. That is
//! the honest answer: whether a module has crashed is modulesd's to say, and
//! filling in a confident `Active` from a file on disk would be a guess
//! rendered as a fact. The live sources layer on top.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::{Extension, ExtensionKind, Health};

/// Where to look. Injectable so discovery is testable against a temp tree
/// rather than whatever is installed on the build machine.
#[derive(Debug, Clone)]
pub struct InventoryRoots {
    /// Enrolled permission profiles, one `.toml` per app.
    pub profiles_dir: PathBuf,
    /// Module directories, system and per-user. Both are read: a module
    /// installed for the user only is as installed as a system one.
    pub module_dirs: Vec<PathBuf>,
    /// Installed bridge directories, each holding a `bridge.toml`.
    ///
    /// Per-user only, because that is the only place bridges are installed:
    /// `arlen_bridge_dir` resolves under `XDG_DATA_HOME` or `$HOME` and has no
    /// system branch. Listing a `/usr/share` root would imply a location
    /// nothing writes to.
    pub bridge_dirs: Vec<PathBuf>,
    /// The ids the user has switched off (`modules.toml`'s disabled list).
    pub disabled_modules: BTreeSet<String>,
}

impl Default for InventoryRoots {
    fn default() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|h| h.join(".config")));
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|h| h.join(".local/share")));

        let mut module_dirs = vec![PathBuf::from("/usr/share/arlen/modules")];
        let mut bridge_dirs = Vec::new();
        if let Some(d) = &data {
            module_dirs.push(d.join("arlen/modules"));
            bridge_dirs.push(d.join("arlen/bridges"));
        }
        Self {
            profiles_dir: config
                .map(|c| c.join("permissions"))
                .unwrap_or_else(|| PathBuf::from("/etc/arlen/permissions")),
            module_dirs,
            bridge_dirs,
            disabled_modules: BTreeSet::new(),
        }
    }
}

/// Read every source and merge. Absence is normal: a machine with no modules
/// and no bridges yields whatever apps are enrolled, never an error.
pub fn read(roots: &InventoryRoots) -> Vec<Extension> {
    crate::merge([apps(roots), modules(roots), bridges(roots)])
}

/// Enrolled apps, from their permission profiles.
///
/// The profile is the right source rather than the installed-package list: it
/// is what actually confines the app, so what it says the app may do is what
/// the app may do. An app with no profile is not enrolled and has no grants to
/// show.
pub fn apps(roots: &InventoryRoots) -> Vec<Extension> {
    let mut out = Vec::new();
    for path in read_dir_sorted(&roots.profiles_dir) {
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let Some(text) = std::fs::read_to_string(&path).ok() else {
            continue;
        };
        let Ok(profile) = toml::from_str::<arlen_permissions::PermissionProfile>(&text) else {
            // A profile that will not parse is not enforcing anything either,
            // so listing it with invented capabilities would be worse than the
            // filename-only row we cannot build. Skipped.
            continue;
        };
        out.push(Extension {
            id: profile.info.app_id.clone(),
            name: profile.info.app_id.clone(),
            kind: ExtensionKind::App,
            capabilities: crate::profile::profile_labels(&profile),
            provenance: Some(format!("{:?}", profile.info.tier).to_lowercase()),
            health: Health::Unknown,
        });
    }
    out
}

/// Installed modules, from their manifests.
///
/// Parsed, not validated: `load_manifest` also checks the entry file exists,
/// and a module whose entry is missing is exactly what someone opens this
/// surface to find. Omitting it would hide the broken thing.
pub fn modules(roots: &InventoryRoots) -> Vec<Extension> {
    let mut out = Vec::new();
    for dir in &roots.module_dirs {
        let system = dir.starts_with("/usr");
        for module_dir in read_dir_sorted(dir) {
            let manifest_path = module_dir.join("module.toml");
            let Some(text) = std::fs::read_to_string(&manifest_path).ok() else {
                continue;
            };
            let Ok(manifest) = arlen_modules::parse_manifest(&text) else {
                continue;
            };
            let id = manifest.module.id.clone();
            let disabled = roots.disabled_modules.contains(&id);
            let entry_present = module_dir.join(&manifest.module.entry).exists();
            out.push(Extension {
                id,
                name: manifest.module.name.clone(),
                kind: ExtensionKind::Module,
                capabilities: crate::module::module_labels(&manifest.capabilities),
                provenance: Some(if system { "system".into() } else { "user".into() }),
                // Switched off beats broken: the user chose the first, and a
                // disabled module's missing entry is not what they are looking
                // for. A present entry still says nothing about whether the
                // module is RUNNING, which only modulesd knows.
                health: if disabled {
                    Health::Disabled
                } else if entry_present {
                    Health::Unknown
                } else {
                    Health::Failed(format!("missing entry file {}", manifest.module.entry))
                },
            });
        }
    }
    out
}

/// Installed bridges, from their configs.
///
/// The namespace is read out of the config text rather than parsed through the
/// bridge crate, so this crate stays free of a daemon dependency. The daemon
/// remains the authority on whether a config is valid; a bridge whose id is
/// unreadable here is listed with no capabilities rather than dropped, because
/// something IS installed and hiding it helps nobody.
pub fn bridges(roots: &InventoryRoots) -> Vec<Extension> {
    let mut out = Vec::new();
    for dir in &roots.bridge_dirs {
        let system = dir.starts_with("/usr");
        for bridge_dir in read_dir_sorted(dir) {
            let config = bridge_dir.join("bridge.toml");
            let Some(text) = std::fs::read_to_string(&config).ok() else {
                continue;
            };
            let namespace = bridge_id(&text).unwrap_or_default();
            let id = if namespace.is_empty() {
                bridge_dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_string()
            } else {
                namespace.clone()
            };
            out.push(Extension {
                name: id.clone(),
                id,
                kind: ExtensionKind::Bridge,
                capabilities: crate::bridge::bridge_labels(&namespace),
                provenance: Some(if system { "system".into() } else { "user".into() }),
                health: Health::Unknown,
            });
        }
    }
    out
}

/// The `[bridge] id` from a config's text.
fn bridge_id(text: &str) -> Option<String> {
    let value: toml::Value = toml::from_str(text).ok()?;
    Some(value.get("bridge")?.get("id")?.as_str()?.to_string())
}

/// Entries of `dir`, sorted, or empty when it does not exist or cannot be read.
/// Sorted because directory order is arbitrary and an inventory that reshuffles
/// between reads cannot be scanned.
fn read_dir_sorted(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn roots_in(dir: &Path) -> InventoryRoots {
        InventoryRoots {
            profiles_dir: dir.join("permissions"),
            module_dirs: vec![dir.join("modules")],
            bridge_dirs: vec![dir.join("bridges")],
            disabled_modules: BTreeSet::new(),
        }
    }

    const MODULE: &str = r#"
[module]
id = "com.example.weather"
name = "Weather"
version = "1.0.0"
description = ""
type = "third-party"
entry = "index.js"
icon = ""

[capabilities]
notifications = true
"#;

    /// A machine with nothing installed is the normal empty case.
    #[test]
    fn an_empty_machine_yields_an_empty_inventory() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read(&roots_in(dir.path())).is_empty());
    }

    #[test]
    fn each_source_contributes_its_own_rows() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path().join("permissions/org.example.App.toml"),
            "[info]\napp_id = \"org.example.App\"\ntier = \"third-party\"\n[network]\nallow_all = true\n",
        );
        write(&dir.path().join("modules/weather/module.toml"), MODULE);
        write(&dir.path().join("modules/weather/index.js"), "//");
        write(
            &dir.path().join("bridges/obsidian/bridge.toml"),
            "[bridge]\nid = \"md.obsidian\"\nallowed_plugin_id = \"x\"\n",
        );

        let all = read(&roots_in(dir.path()));
        assert_eq!(all.len(), 3, "{all:#?}");
        assert_eq!(all[0].kind, ExtensionKind::App);
        assert_eq!(all[0].capabilities, vec!["network".to_string()]);
        assert_eq!(all[1].kind, ExtensionKind::Module);
        assert_eq!(all[1].capabilities, vec!["notifications".to_string()]);
        assert_eq!(all[2].kind, ExtensionKind::Bridge);
        assert_eq!(all[2].capabilities, vec!["write:md.obsidian".to_string()]);
    }

    /// A module whose entry file is gone is exactly what someone opens this
    /// surface to find, so it must be listed as broken rather than omitted.
    #[test]
    fn a_module_with_a_missing_entry_is_listed_as_failed() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("modules/weather/module.toml"), MODULE);
        let rows = modules(&roots_in(dir.path()));
        assert_eq!(rows.len(), 1);
        assert!(
            matches!(&rows[0].health, Health::Failed(why) if why.contains("index.js")),
            "{:?}",
            rows[0].health
        );
    }

    /// The user's own choice is the more useful answer than a broken file.
    #[test]
    fn a_disabled_module_reads_as_disabled_not_broken() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("modules/weather/module.toml"), MODULE);
        let mut roots = roots_in(dir.path());
        roots.disabled_modules.insert("com.example.weather".into());
        assert_eq!(modules(&roots)[0].health, Health::Disabled);
    }

    /// Something IS installed; hiding it because its config is unreadable
    /// helps nobody looking for what is on their machine.
    #[test]
    fn a_bridge_with_an_unreadable_id_is_still_listed_without_authority() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("bridges/broken/bridge.toml"), "not = valid");
        let rows = bridges(&roots_in(dir.path()));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "broken", "falls back to the directory name");
        assert!(rows[0].capabilities.is_empty(), "no authority was claimed");
    }

    /// A profile that will not parse is not confining anything either, so a row
    /// with invented capabilities would be worse than no row.
    #[test]
    fn an_unparseable_profile_is_skipped_rather_than_guessed_at() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("permissions/broken.toml"), "{{{");
        assert!(apps(&roots_in(dir.path())).is_empty());
    }
}

/// What a running modulesd reports about one module.
///
/// Deliberately not the proto type: this crate stays free of daemon
/// dependencies, and the caller that already speaks the socket can map into
/// this in two lines. It also keeps the overlay testable without a daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveModule {
    /// The module id, matching the manifest.
    pub id: String,
    /// Whether the runtime currently admits it.
    pub enabled: bool,
    /// Whether the crash ladder has given up on it.
    pub failed: bool,
    /// What the most recent crash said, when the runtime recorded one.
    pub last_error: Option<String>,
}

/// Replace the disk-derived health of modules the runtime knows about.
///
/// The runtime is authoritative for the modules it has loaded - it is the thing
/// actually running them - so where it answers, its answer wins over anything
/// inferred from files. Crashed beats switched-off, because a module that
/// crashed while enabled is what the user came to find out about.
///
/// A module the runtime does NOT list keeps its disk-derived health. That is
/// not the same as absent: modulesd not knowing about an installed module is
/// itself informative (it was added after the last discovery), and overwriting
/// it with a guessed `Active` would replace a real signal with a wrong one.
///
/// The failure text prefers what the runtime actually recorded. Where it has
/// nothing - a module failed before any reason was captured - it falls back to
/// what is still true, that it crashed enough times to be given up on and the
/// way back is a manual retry. Never a guessed cause.
pub fn overlay_modules(rows: &mut [Extension], live: &[LiveModule]) {
    for row in rows.iter_mut().filter(|r| r.kind == ExtensionKind::Module) {
        let Some(state) = live.iter().find(|l| l.id == row.id) else {
            continue;
        };
        row.health = if state.failed {
            Health::Failed(match &state.last_error {
                Some(why) => format!("{why}; needs a manual retry"),
                None => "crashed repeatedly; needs a manual retry".to_string(),
            })
        } else if !state.enabled {
            Health::Disabled
        } else {
            Health::Active
        };
    }
}

#[cfg(test)]
mod overlay_tests {
    use super::*;

    fn module(id: &str, health: Health) -> Extension {
        Extension {
            id: id.to_string(),
            name: id.to_string(),
            kind: ExtensionKind::Module,
            capabilities: Vec::new(),
            provenance: None,
            health,
        }
    }

    fn live(id: &str, enabled: bool, failed: bool) -> LiveModule {
        LiveModule {
            id: id.to_string(),
            enabled,
            failed,
            last_error: None,
        }
    }

    #[test]
    fn the_runtime_answer_replaces_the_disk_guess() {
        let mut rows = vec![module("a", Health::Unknown), module("b", Health::Unknown)];
        overlay_modules(&mut rows, &[live("a", true, false), live("b", false, false)]);
        assert_eq!(rows[0].health, Health::Active);
        assert_eq!(rows[1].health, Health::Disabled);
    }

    /// A module that crashed while enabled is exactly what the user came to
    /// find out about, so it must not read as merely switched off.
    #[test]
    fn crashed_beats_switched_off() {
        let mut rows = vec![module("a", Health::Unknown)];
        overlay_modules(&mut rows, &[live("a", false, true)]);
        assert!(matches!(rows[0].health, Health::Failed(_)));
    }

    /// modulesd not knowing about an installed module is itself a signal;
    /// replacing it with a guess would destroy it.
    #[test]
    fn a_module_the_runtime_does_not_know_keeps_what_disk_said() {
        let mut rows = vec![module("a", Health::Failed("missing entry file".into()))];
        overlay_modules(&mut rows, &[live("other", true, false)]);
        assert_eq!(rows[0].health, Health::Failed("missing entry file".into()));
    }

    /// The reason the runtime recorded is what the user needs; the generic
    /// fallback is only for a failure that captured none.
    #[test]
    fn the_recorded_reason_is_surfaced_when_there_is_one() {
        let mut rows = vec![module("a", Health::Unknown)];
        let mut state = live("a", true, true);
        state.last_error = Some("execute trapped: unreachable".into());
        overlay_modules(&mut rows, &[state]);
        match &rows[0].health {
            Health::Failed(why) => assert!(why.contains("unreachable"), "{why}"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    /// Apps and bridges are not modulesd's to answer for.
    #[test]
    fn only_module_rows_are_touched() {
        let mut rows = vec![Extension {
            kind: ExtensionKind::App,
            ..module("a", Health::Unknown)
        }];
        overlay_modules(&mut rows, &[live("a", true, false)]);
        assert_eq!(rows[0].health, Health::Unknown);
    }
}
