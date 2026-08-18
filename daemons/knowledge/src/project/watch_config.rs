//! Watch configuration for project detection.
//!
//! Loaded from `~/.config/arlen/graph.toml` `[projects]` section.
//! Falls back to defaults if the file is missing or unparseable.

use serde::Deserialize;
use std::path::PathBuf;

/// `[projects]` section from `graph.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct WatchConfig {
    /// Directories to scan for projects (supports `~`).
    #[serde(default = "default_watch_dirs")]
    pub watch_directories: Vec<String>,

    /// Maximum recursion depth when scanning.
    #[serde(default = "default_max_depth")]
    pub max_depth: usize,

    /// Auto-promote an inferred project after this many distinct
    /// files have been opened in one session. Lower = faster
    /// promotion, more noise; higher = slower, only well-used
    /// projects surface in Waypointer / Focus Mode. Was a
    /// hardcoded `3` until Sprint C made it user-configurable.
    #[serde(default = "default_auto_promote_threshold")]
    pub auto_promote_threshold: usize,
}

fn default_watch_dirs() -> Vec<String> {
    vec![
        "~/Projects".into(),
        "~/Repositories".into(),
        "~/Documents".into(),
        "~/Developer".into(),
        "~/Code".into(),
    ]
}

fn default_max_depth() -> usize {
    3
}

fn default_auto_promote_threshold() -> usize {
    3
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            watch_directories: default_watch_dirs(),
            max_depth: default_max_depth(),
            auto_promote_threshold: default_auto_promote_threshold(),
        }
    }
}

/// Top-level `graph.toml` structure.
#[derive(Debug, Clone, Default, Deserialize)]
struct GraphConfig {
    // `Option`, not `#[serde(default)]`, so a file that parses without a
    // `[projects]` section is distinguishable from one that sets it. Both end up
    // watching the same three home directories; only one of them says so, and
    // until 18 August that was the missing-file case alone. A config that exists
    // and simply does not mention projects took the defaults in silence - which
    // is how a hermetic run of this daemon ended up scanning the real
    // `~/Repositories`, with a `watch_directories = []` in the file and no
    // section header above it.
    projects: Option<WatchConfig>,
}

impl WatchConfig {
    /// Load from `~/.config/arlen/graph.toml`.
    /// Returns defaults if the file is missing or invalid.
    pub fn load() -> Self {
        let Some(path) = dirs::config_dir().map(|p| p.join("arlen/graph.toml")) else {
            tracing::debug!("no config dir found, using defaults");
            return Self::default();
        };

        if !path.exists() {
            // At INFO, not debug, and naming the directories: with no config this
            // daemon starts scanning three directories in the user's home, and the
            // journal should say so rather than leave a surprising scan
            // unexplained. Found by running it headless with an empty config home,
            // where it read `~/Documents`, `~/Projects` and `~/Repositories`
            // without a word at the level the units log at.
            let d = Self::default();
            tracing::info!(
                "{} not found; watching the default user directories: {}",
                path.display(),
                d.watch_directories.join(", ")
            );
            return d;
        }

        match std::fs::read_to_string(&path) {
            Ok(content) => match toml::from_str::<GraphConfig>(&content) {
                Ok(gc) => match gc.projects {
                    Some(projects) => {
                        tracing::info!("loaded project config from {}", path.display());
                        projects
                    }
                    None => {
                        let d = Self::default();
                        tracing::info!(
                            "{} has no [projects] section; watching the default user \
                             directories: {}",
                            path.display(),
                            d.watch_directories.join(", ")
                        );
                        d
                    }
                },
                Err(e) => {
                    tracing::warn!("failed to parse {}: {e}, using defaults", path.display());
                    Self::default()
                }
            },
            Err(e) => {
                tracing::warn!("failed to read {}: {e}, using defaults", path.display());
                Self::default()
            }
        }
    }

    /// Expand `~` and filter to existing directories.
    pub fn expanded_directories(&self) -> Vec<PathBuf> {
        self.watch_directories
            .iter()
            .filter_map(|dir| {
                let expanded = shellexpand::tilde(dir);
                let path = PathBuf::from(expanded.as_ref());
                if path.is_dir() {
                    Some(path)
                } else {
                    tracing::debug!("watch directory does not exist: {dir}");
                    None
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_entries() {
        let cfg = WatchConfig::default();
        assert!(!cfg.watch_directories.is_empty());
        assert_eq!(cfg.max_depth, 3);
    }

    #[test]
    fn parse_custom_config() {
        let toml = r#"
[projects]
watch_directories = ["/tmp/projects"]
max_depth = 2
"#;
        let gc: GraphConfig = toml::from_str(toml).unwrap();
        let projects = gc.projects.expect("the section is present");
        assert_eq!(projects.watch_directories, vec!["/tmp/projects"]);
        assert_eq!(projects.max_depth, 2);
    }

    /// A file with no `[projects]` section leaves it ABSENT rather than
    /// defaulted, so `load` can say out loud that it is about to watch three
    /// directories in the user's home. Both cases end up watching them; only a
    /// distinguishable one can be reported.
    #[test]
    fn a_file_without_the_section_leaves_it_absent() {
        let gc: GraphConfig = toml::from_str("").unwrap();
        assert!(gc.projects.is_none());
        let d = WatchConfig::default();
        assert!(!d.watch_directories.is_empty(), "and the fallback still watches them");
        assert_eq!(d.max_depth, 3);
    }

    /// The shape that produced the silence: a `watch_directories` at the top
    /// level rather than under `[projects]`. It parses, it looks like it turned
    /// watching off, and it does the opposite.
    #[test]
    fn a_key_outside_the_section_does_not_configure_it() {
        let gc: GraphConfig = toml::from_str("watch_directories = []\n").unwrap();
        assert!(gc.projects.is_none(), "the section, not the key, is what sets this");
    }

    /// Sprint C added `auto_promote_threshold`. Existing user
    /// graph.toml files without the field must still parse and
    /// fall back to 3 — otherwise upgrading would crash the
    /// daemon on first start.
    #[test]
    fn missing_threshold_falls_back_to_default() {
        let toml = r#"
[projects]
watch_directories = ["/tmp/projects"]
max_depth = 2
"#;
        let gc: GraphConfig = toml::from_str(toml).unwrap();
        assert_eq!(
            gc.projects.expect("section present").auto_promote_threshold, 3,
            "missing threshold must default to 3 (compositor #29 era \
             behaviour) — change with care, this affects every existing \
             user's graph.toml"
        );
    }

    #[test]
    fn explicit_threshold_overrides_default() {
        let toml = r#"
[projects]
auto_promote_threshold = 7
"#;
        let gc: GraphConfig = toml::from_str(toml).unwrap();
        assert_eq!(gc.projects.expect("section present").auto_promote_threshold, 7);
    }
}
