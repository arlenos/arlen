//! Where the `mimeapps.list` files are, and in which order they win.
//!
//! Separate from the lookup in [`super::mimeapps`] because they fail
//! differently: getting the order wrong means the user's own choice loses to a
//! packaged default silently, and that is only visible if the order itself is
//! testable. So this computes the list of paths and touches nothing - the host
//! reads whichever of them exist.
//!
//! Spec: <https://specifications.freedesktop.org/mime-apps-spec/latest/>

use std::path::PathBuf;

/// The XDG environment this resolution depends on, as read from the process.
///
/// Raw strings rather than resolved paths, so the defaulting rules are part of
/// what the tests cover: an empty or unset variable falls back to the spec's
/// default, and those defaults are the difference between reading the user's
/// choices and reading none of them.
#[derive(Debug, Default, Clone)]
pub struct XdgEnv {
    /// `$HOME`, for the two defaults that are relative to it.
    pub home: Option<String>,
    /// `$XDG_CONFIG_HOME`, default `$HOME/.config`.
    pub config_home: Option<String>,
    /// `$XDG_CONFIG_DIRS`, default `/etc/xdg`.
    pub config_dirs: Option<String>,
    /// `$XDG_DATA_HOME`, default `$HOME/.local/share`.
    pub data_home: Option<String>,
    /// `$XDG_DATA_DIRS`, default `/usr/local/share:/usr/share`.
    pub data_dirs: Option<String>,
    /// `$XDG_CURRENT_DESKTOP`, a colon-separated list, most specific first.
    pub current_desktop: Option<String>,
}

impl XdgEnv {
    /// Read the variables this needs from the process environment.
    pub fn from_process() -> Self {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        Self {
            home: var("HOME"),
            config_home: var("XDG_CONFIG_HOME"),
            config_dirs: var("XDG_CONFIG_DIRS"),
            data_home: var("XDG_DATA_HOME"),
            data_dirs: var("XDG_DATA_DIRS"),
            current_desktop: var("XDG_CURRENT_DESKTOP"),
        }
    }

    fn under_home(&self, tail: &str) -> Option<PathBuf> {
        self.home.as_ref().map(|h| PathBuf::from(h).join(tail))
    }

    fn config_home_dir(&self) -> Option<PathBuf> {
        match &self.config_home {
            Some(v) => Some(PathBuf::from(v)),
            None => self.under_home(".config"),
        }
    }

    fn data_home_dir(&self) -> Option<PathBuf> {
        match &self.data_home {
            Some(v) => Some(PathBuf::from(v)),
            None => self.under_home(".local/share"),
        }
    }

    fn desktops(&self) -> Vec<String> {
        self.current_desktop
            .as_deref()
            .unwrap_or_default()
            .split(':')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_lowercase)
            .collect()
    }
}

fn split_dirs(value: Option<&String>, fallback: &str) -> Vec<PathBuf> {
    let raw = value.map(String::as_str).unwrap_or(fallback);
    raw.split(':')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// The `mimeapps.list` paths to consult, highest precedence first.
///
/// In each directory the desktop-specific file comes before the plain one, so a
/// name in `$XDG_CURRENT_DESKTOP` can override a general choice; where that
/// variable lists several desktops they are tried in the order given, which is
/// the order the session declared them in.
///
/// The config directories come before the data directories throughout, because
/// the config ones hold what the user chose and the data ones hold what packages
/// shipped. A returned path need not exist - the caller reads the ones that do.
pub fn mimeapps_paths(env: &XdgEnv) -> Vec<PathBuf> {
    let desktops = env.desktops();
    let mut out = Vec::new();

    // The file names to try inside one directory, most specific first.
    let names: Vec<String> = desktops
        .iter()
        .map(|d| format!("{d}-mimeapps.list"))
        .chain(std::iter::once("mimeapps.list".to_string()))
        .collect();

    let mut push_all = |dir: PathBuf| {
        for n in &names {
            out.push(dir.join(n));
        }
    };

    if let Some(d) = env.config_home_dir() {
        push_all(d);
    }
    for d in split_dirs(env.config_dirs.as_ref(), "/etc/xdg") {
        push_all(d);
    }
    // Under the data directories the files live beside the desktop entries.
    if let Some(d) = env.data_home_dir() {
        push_all(d.join("applications"));
    }
    for d in split_dirs(env.data_dirs.as_ref(), "/usr/local/share:/usr/share") {
        push_all(d.join("applications"));
    }
    out
}

/// Read the handler files that exist, in precedence order.
///
/// The one piece of this module that touches the disk, kept here rather than in
/// the host because the alternative is every caller re-deriving "read the paths
/// `mimeapps_paths` returned, skip the ones that are not there, keep the order".
/// A path that cannot be read is skipped rather than failing the lookup: a
/// handler map is a merge of whatever is present, and one unreadable file in
/// `/etc/xdg` should not cost the user their own choices.
pub fn load_mimeapps(env: &XdgEnv) -> Vec<super::mimeapps::MimeApps> {
    mimeapps_paths(env)
        .into_iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .map(|text| super::mimeapps::parse(&text))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> XdgEnv {
        XdgEnv {
            home: Some("/home/u".into()),
            ..Default::default()
        }
    }

    fn strs(paths: &[PathBuf]) -> Vec<String> {
        paths.iter().map(|p| p.display().to_string()).collect()
    }

    #[test]
    fn the_defaults_are_the_spec_defaults() {
        assert_eq!(
            strs(&mimeapps_paths(&env())),
            vec![
                "/home/u/.config/mimeapps.list",
                "/etc/xdg/mimeapps.list",
                "/home/u/.local/share/applications/mimeapps.list",
                "/usr/local/share/applications/mimeapps.list",
                "/usr/share/applications/mimeapps.list",
            ]
        );
    }

    /// What the user chose beats what a package shipped, which is the whole
    /// reason the order exists.
    #[test]
    fn config_comes_before_data() {
        let paths = strs(&mimeapps_paths(&env()));
        let config = paths
            .iter()
            .position(|p| p.starts_with("/home/u/.config"))
            .unwrap();
        let data = paths
            .iter()
            .position(|p| p.starts_with("/home/u/.local/share"))
            .unwrap();
        assert!(config < data);
    }

    #[test]
    fn an_explicit_config_home_replaces_the_default() {
        let e = XdgEnv {
            config_home: Some("/tmp/cfg".into()),
            ..env()
        };
        assert_eq!(strs(&mimeapps_paths(&e))[0], "/tmp/cfg/mimeapps.list");
    }

    #[test]
    fn the_desktop_specific_file_comes_first_in_each_directory() {
        let e = XdgEnv {
            current_desktop: Some("Arlen".into()),
            ..env()
        };
        assert_eq!(
            strs(&mimeapps_paths(&e))[..2],
            [
                "/home/u/.config/arlen-mimeapps.list",
                "/home/u/.config/mimeapps.list"
            ]
        );
    }

    /// A session declaring several desktops declared them in an order; keep it.
    #[test]
    fn several_desktops_are_tried_in_the_declared_order() {
        let e = XdgEnv {
            current_desktop: Some("Arlen:GNOME".into()),
            ..env()
        };
        assert_eq!(
            strs(&mimeapps_paths(&e))[..3],
            [
                "/home/u/.config/arlen-mimeapps.list",
                "/home/u/.config/gnome-mimeapps.list",
                "/home/u/.config/mimeapps.list"
            ]
        );
    }

    #[test]
    fn every_search_directory_gets_the_desktop_specific_name_too() {
        let e = XdgEnv {
            current_desktop: Some("Arlen".into()),
            ..env()
        };
        let paths = strs(&mimeapps_paths(&e));
        assert!(paths.contains(&"/usr/share/applications/arlen-mimeapps.list".to_string()));
        assert!(paths.contains(&"/etc/xdg/arlen-mimeapps.list".to_string()));
    }

    #[test]
    fn multiple_config_dirs_keep_their_order() {
        let e = XdgEnv {
            config_dirs: Some("/a:/b".into()),
            ..env()
        };
        let paths = strs(&mimeapps_paths(&e));
        let a = paths.iter().position(|p| p == "/a/mimeapps.list").unwrap();
        let b = paths.iter().position(|p| p == "/b/mimeapps.list").unwrap();
        assert!(a < b);
    }

    /// An empty entry in a colon list is not the current directory.
    #[test]
    fn empty_segments_are_dropped_rather_than_becoming_paths() {
        let e = XdgEnv {
            config_dirs: Some("/a::/b:".into()),
            current_desktop: Some("Arlen::".into()),
            ..env()
        };
        let paths = strs(&mimeapps_paths(&e));
        assert!(!paths.iter().any(|p| p.starts_with("mimeapps")));
        assert_eq!(
            paths
                .iter()
                .filter(|p| p.contains("-mimeapps.list"))
                .count(),
            paths.len() / 2
        );
    }

    /// Without `$HOME` the two home-relative directories cannot be named, and
    /// guessing one would point the lookup at a path belonging to nobody. The
    /// system-wide ones still work.
    #[test]
    fn a_missing_home_drops_only_the_home_directories() {
        let e = XdgEnv::default();
        assert_eq!(
            strs(&mimeapps_paths(&e)),
            vec![
                "/etc/xdg/mimeapps.list",
                "/usr/local/share/applications/mimeapps.list",
                "/usr/share/applications/mimeapps.list",
            ]
        );
    }

    /// The order on disk is the order in the list, and a path that is not there
    /// is skipped rather than becoming an empty entry that shifts the rest.
    #[test]
    fn loading_keeps_the_order_and_skips_what_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::write(
            cfg.join("mimeapps.list"),
            "[Default Applications]\ntext/plain=mine.desktop;\n",
        )
        .unwrap();
        let e = XdgEnv {
            config_home: Some(cfg.display().to_string()),
            // Points at nothing, so these contribute no entries at all.
            config_dirs: Some(dir.path().join("absent").display().to_string()),
            data_dirs: Some(dir.path().join("gone").display().to_string()),
            ..env()
        };
        let loaded = load_mimeapps(&e);
        assert_eq!(loaded.len(), 1);
        assert_eq!(
            super::super::mimeapps::default_handler(&loaded, "text/plain", |_| true).as_deref(),
            Some("mine.desktop")
        );
    }

    #[test]
    fn loading_a_tree_with_no_handler_files_is_empty_rather_than_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let e = XdgEnv {
            config_home: Some(dir.path().display().to_string()),
            config_dirs: Some(dir.path().display().to_string()),
            data_home: Some(dir.path().display().to_string()),
            data_dirs: Some(dir.path().display().to_string()),
            ..env()
        };
        assert!(load_mimeapps(&e).is_empty());
    }

    #[test]
    fn desktop_names_are_lowercased_for_the_file_name() {
        let e = XdgEnv {
            current_desktop: Some("KDE".into()),
            ..env()
        };
        assert_eq!(
            strs(&mimeapps_paths(&e))[0],
            "/home/u/.config/kde-mimeapps.list"
        );
    }
}
