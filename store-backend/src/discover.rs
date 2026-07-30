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
