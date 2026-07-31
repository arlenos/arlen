//! SC-5: finding the catalog sources the compose step reads.
//!
//! `main.rs` took three env paths that nothing set, so on a real image every
//! source was absent and the catalog was empty. The metadata is already on the
//! machine at well-known locations; this locates it.
//!
//! **Discovery reports what it finds; it does not read or parse.** Returning
//! paths keeps this pure enough to test against a temp tree, and leaves the
//! gzip-aware reading in one place rather than duplicated per source.
//!
//! Absence is normal and never an error. A machine with no Flatpak installation,
//! no Debian catalog, or no enrolled apps yields an empty list for that source
//! and a catalog composed from whatever else is there.

use std::path::{Path, PathBuf};

/// Where to look. Injectable so the discovery is testable against a temp tree
/// rather than whatever happens to be installed on the build machine.
#[derive(Debug, Clone)]
pub struct SourceRoots {
    /// Flatpak installation roots, system and per-user. Both are searched: an
    /// app installed for the user only is as real as a system one.
    pub flatpak_dirs: Vec<PathBuf>,
    /// Directories holding Debian DEP-11 catalogs. `swcatalog` is the current
    /// name, `app-info` the older one; both appear in the field depending on the
    /// release, so both are searched.
    pub dep11_dirs: Vec<PathBuf>,
    /// Where enrolled permission profiles live.
    pub profiles_dir: PathBuf,
}

impl Default for SourceRoots {
    fn default() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let mut flatpak_dirs = vec![PathBuf::from("/var/lib/flatpak")];
        let mut dep11_dirs = vec![
            PathBuf::from("/var/lib/swcatalog"),
            PathBuf::from("/var/lib/app-info"),
        ];
        if let Some(h) = &home {
            flatpak_dirs.push(h.join(".local/share/flatpak"));
            dep11_dirs.push(h.join(".local/share/swcatalog"));
        }
        Self {
            flatpak_dirs,
            dep11_dirs,
            profiles_dir: home
                .map(|h| h.join(".config/permissions"))
                .unwrap_or_else(|| PathBuf::from("/etc/arlen/permissions")),
        }
    }
}

/// What was found. Paths only - the caller reads them.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Discovered {
    /// Flatpak remote AppStream catalogs (`.xml.gz`), one per remote.
    pub flathub_xml: Vec<PathBuf>,
    /// Debian DEP-11 catalogs (`.yml.gz`), one per component/suite.
    pub dep11_yaml: Vec<PathBuf>,
    /// `(app-id, metadata path)` per installed Flatpak app. The id comes from
    /// Flatpak's own directory layout, so it is read, not guessed.
    pub flatpak_metadata: Vec<(String, PathBuf)>,
    /// `(app-id, profile path)` per enrolled app, the id being the filename stem.
    pub apt_profiles: Vec<(String, PathBuf)>,
}

/// Locate every source under `roots`.
pub fn discover(roots: &SourceRoots) -> Discovered {
    let mut found = Discovered::default();

    for dir in &roots.flatpak_dirs {
        // `<root>/appstream/<remote>/<arch>/active/appstream.xml[.gz]`
        for remote in read_dir_sorted(&dir.join("appstream")) {
            for arch in read_dir_sorted(&remote) {
                let active = arch.join("active");
                for name in ["appstream.xml.gz", "appstream.xml"] {
                    let candidate = active.join(name);
                    if candidate.is_file() {
                        found.flathub_xml.push(candidate);
                        break; // One per arch; prefer the compressed form.
                    }
                }
            }
        }
        // `<root>/app/<app-id>/current/active/metadata` - the id IS the directory
        // name, which is why this mapping is not a guess.
        for app in read_dir_sorted(&dir.join("app")) {
            let Some(id) = app.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let metadata = app.join("current/active/metadata");
            if metadata.is_file() {
                found.flatpak_metadata.push((id.to_string(), metadata));
            }
        }
    }

    for dir in &roots.dep11_dirs {
        // `<root>/yaml/<suite>_<component>.yml[.gz]`, flat.
        for entry in read_dir_sorted(&dir.join("yaml")) {
            let name = entry.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if entry.is_file() && (name.ends_with(".yml") || name.ends_with(".yml.gz")) {
                found.dep11_yaml.push(entry);
            }
        }
    }

    for entry in read_dir_sorted(&roots.profiles_dir) {
        if entry.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        if let Some(stem) = entry.file_stem().and_then(|s| s.to_str()) {
            found.apt_profiles.push((stem.to_string(), entry));
        }
    }

    found
}

/// Entries of `dir`, sorted, or empty when it does not exist or cannot be read.
///
/// Sorted because directory order is arbitrary: an unsorted catalog list would
/// make the composed result depend on filesystem iteration order, so two runs on
/// the same machine could merge sources in a different order.
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

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "x").unwrap();
    }

    fn roots_in(dir: &Path) -> SourceRoots {
        SourceRoots {
            flatpak_dirs: vec![dir.join("flatpak")],
            dep11_dirs: vec![dir.join("swcatalog")],
            profiles_dir: dir.join("permissions"),
        }
    }

    /// A machine with nothing installed is the normal empty case, not an error.
    #[test]
    fn an_absent_or_corrupt_lock_reads_as_nothing_installed() {
        // Both are normal-enough states, and the only consumer is the update
        // check: not noticing an update beats a store that will not open.
        assert!(super::parse_lock("").is_empty());
        assert!(super::parse_lock("this is not toml {{{").is_empty());
    }

    #[test]
    fn an_installed_entry_is_read_with_its_layer_and_version() {
        let entries = super::parse_lock(
            "[entries.\"org.example.App\"]\ncomponent_id = \"org.example.App\"\n\
             source_layer = \"official\"\nversion = \"1.2.0\"\n",
        );
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].version, "1.2.0");
        assert_eq!(entries[0].source_layer, "official");
    }

    /// An entry missing a field it would be compared on is skipped, not
    /// defaulted - comparing a guessed version is worse than not comparing.
    #[test]
    fn an_incomplete_entry_is_not_compared() {
        assert!(super::parse_lock(
            "[entries.\"org.example.App\"]\ncomponent_id = \"org.example.App\"\n"
        )
        .is_empty());
    }

    /// An unknown layer is dropped: offering an update from the wrong layer
    /// would install a different app than the one on disk.
    #[test]
    fn an_unknown_layer_is_dropped_rather_than_guessed() {
        assert!(super::parse_layer("official").is_some());
        assert!(super::parse_layer("snap").is_none());
        assert!(super::parse_layer("").is_none());
    }

    #[test]
    fn nothing_installed_finds_nothing_and_does_not_fail() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(discover(&roots_in(dir.path())), Discovered::default());
    }

    #[test]
    fn a_flatpak_app_yields_its_id_and_metadata_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("flatpak");
        touch(&root.join("app/org.gnome.Calculator/current/active/metadata"));
        touch(&root.join("appstream/flathub/x86_64/active/appstream.xml.gz"));

        let found = discover(&roots_in(dir.path()));
        assert_eq!(found.flatpak_metadata.len(), 1);
        assert_eq!(found.flatpak_metadata[0].0, "org.gnome.Calculator");
        assert_eq!(found.flathub_xml.len(), 1);
    }

    /// An app directory with no metadata file is skipped rather than reported
    /// with a path that does not exist.
    #[test]
    fn an_app_without_metadata_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("flatpak/app/org.example.Half")).unwrap();
        assert!(discover(&roots_in(dir.path())).flatpak_metadata.is_empty());
    }

    #[test]
    fn dep11_catalogs_are_found_compressed_or_not() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = dir.path().join("swcatalog/yaml");
        touch(&yaml.join("trixie_main.yml.gz"));
        touch(&yaml.join("trixie_contrib.yml"));
        touch(&yaml.join("README"));

        let found = discover(&roots_in(dir.path()));
        assert_eq!(found.dep11_yaml.len(), 2, "{:?}", found.dep11_yaml);
    }

    #[test]
    fn enrolled_profiles_yield_their_app_id() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("permissions/org.example.App.toml"));
        touch(&dir.path().join("permissions/notes.txt"));

        let found = discover(&roots_in(dir.path()));
        assert_eq!(found.apt_profiles.len(), 1);
        assert_eq!(found.apt_profiles[0].0, "org.example.App");
    }

    /// Directory order is arbitrary, so two runs on one machine must not compose
    /// their sources in a different order.
    #[test]
    fn discovery_is_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = dir.path().join("swcatalog/yaml");
        for n in ["c.yml", "a.yml", "b.yml"] {
            touch(&yaml.join(n));
        }
        let first = discover(&roots_in(dir.path()));
        assert_eq!(first, discover(&roots_in(dir.path())));
        let names: Vec<&str> = first
            .dep11_yaml
            .iter()
            .filter_map(|p| p.file_name()?.to_str())
            .collect();
        assert_eq!(names, vec!["a.yml", "b.yml", "c.yml"]);
    }
}

/// What installd recorded about one installed package.
///
/// A deliberate partial read of a file this crate does not own: installd's lock
/// carries more per entry (granted capabilities, recipe commit, revision) and
/// this takes only the three fields an update check needs. Depending on the
/// installd crate to get them would pull a D-Bus daemon into the store for a
/// struct, and the store has no business knowing the rest.
///
/// The fields are `#[serde(default)]`-free on purpose: an entry missing its id,
/// layer or version is not an entry this can compare, and silently defaulting
/// one would compare the wrong thing.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct InstalledEntry {
    /// The component this is an installation of.
    pub component_id: String,
    /// Which layer it came from, as installd wrote it.
    pub source_layer: String,
    /// The version on disk.
    pub version: String,
}

#[derive(Debug, Default, serde::Deserialize)]
struct LockFile {
    #[serde(default)]
    entries: std::collections::BTreeMap<String, InstalledEntry>,
}

/// `~/.local/share/arlen/apps/installed.lock`, matching installd's `lock_path`.
pub fn lock_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("arlen").join("apps").join("installed.lock"))
}

/// `~/.local/share/arlen/apps/skipped-updates.toml`, beside the lock it
/// qualifies.
///
/// Its own file rather than a field on the lock entry: the lock records what an
/// install did, and a skip is the user declining something that has not happened.
/// Rewriting the lock to hold a preference would also mean the update check
/// writes to the record the capability gate reads as its old side.
pub fn skipped_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("arlen").join("apps").join("skipped-updates.toml"))
}

/// Which offered version the user has skipped, per component-id.
///
/// Absent or unparseable yields nothing, which shows an update the user had
/// skipped. That is the right direction to fail: a skip the system forgets is a
/// row reappearing, while a skip it invents hides an update that exists.
pub fn skipped_updates() -> std::collections::BTreeMap<String, String> {
    let Some(text) = skipped_path().and_then(|p| std::fs::read_to_string(p).ok()) else {
        return Default::default();
    };
    parse_skipped(&text)
}

/// Parse the skip file's text, so the reading is testable without a file.
fn parse_skipped(text: &str) -> std::collections::BTreeMap<String, String> {
    toml::from_str(text).unwrap_or_default()
}

/// Record that `version` of `id` was skipped, replacing any earlier skip for it.
///
/// Atomic, because the update check reads this file and a half-written one would
/// parse as no skips at all.
pub fn skip_update(id: &str, version: &str) -> Result<(), String> {
    let path = skipped_path().ok_or_else(|| "no data directory to record the skip in".to_string())?;
    let mut skipped = skipped_updates();
    skipped.insert(id.to_string(), version.to_string());
    let body = toml::to_string(&skipped).map_err(|e| format!("could not serialise the skips: {e}"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, body).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(())
}

/// Read what is installed, or nothing.
///
/// An absent lock means nothing has been installed through installd, which is a
/// normal state. A CORRUPT lock also yields nothing rather than an error,
/// because the only consumer is the update check: failing to notice an update
/// is a smaller harm than a store that will not open.
pub fn installed_entries() -> Vec<InstalledEntry> {
    let Some(text) = lock_path().and_then(|p| std::fs::read_to_string(p).ok()) else {
        return Vec::new();
    };
    parse_lock(&text)
}

/// Parse a lock's text, so the reading is testable without a file.
fn parse_lock(text: &str) -> Vec<InstalledEntry> {
    toml::from_str::<LockFile>(text)
        .map(|l| l.entries.into_values().collect())
        .unwrap_or_default()
}

/// What is installed, in the shape [`crate::query::Request::Outdated`] wants.
///
/// The join that was missing: the wire op and the comparison both existed, and
/// nothing built the map they need. Entries whose layer is not one this store
/// knows are dropped rather than guessed at - an update offered from the wrong
/// layer would install a different app than the one on disk.
pub fn installed_versions(
) -> std::collections::BTreeMap<String, crate::query::InstalledVersion> {
    versions_from(installed_entries())
}

/// The capability labels each installed app currently HOLDS, from the permission
/// profile in force for it.
///
/// The profile rather than the catalog, because the catalog describes what a
/// source offers today and the question is what this machine already granted. An
/// app with no readable profile is simply absent from the map, which the update
/// row carries through as "not known" rather than as "holds nothing" - the latter
/// would make every capability of every update look newly requested.
pub fn held_capabilities() -> std::collections::BTreeMap<String, Vec<String>> {
    installed_entries()
        .into_iter()
        .filter_map(|e| {
            let profile = arlen_permissions::load_profile(&e.component_id).ok()?;
            Some((
                e.component_id,
                arlen_extensions::profile::profile_labels(&profile),
            ))
        })
        .collect()
}

/// The join itself, over entries already read, so the mapping is testable
/// without a lock file or an environment.
fn versions_from(
    entries: Vec<InstalledEntry>,
) -> std::collections::BTreeMap<String, crate::query::InstalledVersion> {
    entries
        .into_iter()
        .filter_map(|e| {
            let layer = parse_layer(&e.source_layer)?;
            Some((
                e.component_id,
                crate::query::InstalledVersion {
                    layer,
                    version: e.version,
                },
            ))
        })
        .collect()
}

/// The layer installd recorded, as this store's enum.
///
/// Matched explicitly rather than through serde so an unknown string is a
/// `None` we can drop, not a parse error that would fail the whole read.
fn parse_layer(name: &str) -> Option<crate::catalog::SourceLayer> {
    use crate::catalog::SourceLayer as L;
    Some(match name {
        "personal" => L::Personal,
        "community" => L::Community,
        "official" => L::Official,
        "flatpak" => L::Flatpak,
        "apt" => L::Apt,
        _ => return None,
    })
}

#[cfg(test)]
mod lock_tests {
    use super::*;

    fn entry(id: &str, layer: &str, version: &str) -> InstalledEntry {
        InstalledEntry {
            component_id: id.to_string(),
            source_layer: layer.to_string(),
            version: version.to_string(),
        }
    }

    #[test]
    fn a_lock_becomes_the_map_the_update_check_compares_against() {
        let map = versions_from(vec![
            entry("org.x.Chat", "apt", "1.0"),
            entry("org.x.Paint", "flatpak", "2.3"),
        ]);
        assert_eq!(map.len(), 2);
        assert_eq!(map["org.x.Chat"].version, "1.0");
        assert_eq!(map["org.x.Paint"].layer, crate::catalog::SourceLayer::Flatpak);
    }

    #[test]
    fn an_entry_from_a_layer_this_store_does_not_know_is_dropped() {
        // Guessing a layer would offer an update built by a source the user never
        // chose, which installs a different app than the one on disk. Dropping it
        // means the app simply shows no update, which is recoverable.
        let map = versions_from(vec![
            entry("org.x.Chat", "snap", "1.0"),
            entry("org.x.Paint", "apt", "2.0"),
        ]);
        assert_eq!(map.len(), 1);
        assert!(map.contains_key("org.x.Paint"));
    }

    #[test]
    fn a_corrupt_lock_reads_as_nothing_installed() {
        // Deliberately not an error: the only consumer is the update check, and a
        // store that refuses to open is worse than one that misses an update.
        assert!(parse_lock("this is not toml {{{").is_empty());
    }
}
