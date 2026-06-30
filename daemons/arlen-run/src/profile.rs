//! Mapping a permission profile to the confiner's inputs.
//!
//! `arlen-run` reads an app's [`PermissionProfile`] and derives the writable
//! filesystem set and the network policy the confiner needs: the app's own state
//! dirs are always writable, the `[filesystem]` flags add the matching XDG user
//! dirs, `custom` paths are added verbatim, and `[network]` maps to a
//! [`NetworkPolicy`]. The result feeds `arlen_confiner::app_runtime_profile`.

use std::path::{Path, PathBuf};

use arlen_confiner::NetworkPolicy;
use arlen_permissions::{FilesystemPermissions, NetworkPermissions};

/// The resolved XDG user directories (the launcher resolves them once; the mapping
/// is pure over them so it is testable without touching the real home).
#[derive(Debug, Clone)]
pub struct UserDirs {
    /// `~/Documents`.
    pub documents: PathBuf,
    /// `~/Downloads`.
    pub downloads: PathBuf,
    /// `~/Pictures`.
    pub pictures: PathBuf,
    /// `~/Music`.
    pub music: PathBuf,
    /// `~/Videos`.
    pub videos: PathBuf,
}

/// The confiner inputs derived from a profile: the read-write set and the network
/// policy. `arlen-run` passes these to `app_runtime_profile`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfinementInputs {
    /// Directories the app may write (its own state dirs, the flag-gated XDG dirs,
    /// and any `custom` paths). Each becomes a read-write bind.
    pub app_dirs: Vec<PathBuf>,
    /// The network policy.
    pub network: NetworkPolicy,
}

/// The host filesystem roots a `custom` grant may never bind. Binding any of
/// these (or `/`, or an ancestor of the home) is the `--filesystem=host` escape
/// that defeats the portal-mediated FS model (same-uid-isolation-plan.md
/// Tier-A #3): it hands a confined app the OS + every user's data. Arlen's
/// profile format simply does not offer it - a `custom` entry resolving to one
/// of these is dropped, not bound. A specific subdirectory (under the home, a
/// project dir, a data mount) is unaffected; only the whole-tree roots are.
const FORBIDDEN_FS_ROOTS: &[&str] = &[
    "/", "/etc", "/usr", "/var", "/boot", "/bin", "/sbin", "/lib", "/lib64",
    "/proc", "/sys", "/dev", "/run", "/root",
];

/// Whether `path` is a host-filesystem escape a `custom` grant must not bind:
/// one of the [`FORBIDDEN_FS_ROOTS`], or an ancestor of `home` (e.g. `/home`,
/// which would expose every user's home, or `/`). A specific subdirectory of
/// the home (e.g. `~/Projects`) is NOT an escape.
pub fn is_host_escape(path: &Path, home: &Path) -> bool {
    FORBIDDEN_FS_ROOTS.iter().any(|r| path == Path::new(r)) || home.starts_with(path)
}

/// Map an app's filesystem + network permissions to the confiner inputs. The app's
/// own state dirs (`~/.local/share|.config|.cache/arlen/apps/{app_id}`) are always
/// writable so the app can function; the `home`/`documents`/... flags add the
/// matching user dirs; `custom` paths are added verbatim EXCEPT a host-filesystem
/// escape ([`is_host_escape`]), which is dropped (portal-only-FS, Tier-A #3).
pub fn confinement_inputs(
    fs: &FilesystemPermissions,
    net: &NetworkPermissions,
    app_id: &str,
    home: &Path,
    dirs: &UserDirs,
) -> ConfinementInputs {
    let mut app_dirs = vec![
        home.join(".local/share/arlen/apps").join(app_id),
        home.join(".config/arlen/apps").join(app_id),
        home.join(".cache/arlen/apps").join(app_id),
    ];
    if fs.home {
        app_dirs.push(home.to_path_buf());
    }
    if fs.documents {
        app_dirs.push(dirs.documents.clone());
    }
    if fs.downloads {
        app_dirs.push(dirs.downloads.clone());
    }
    if fs.pictures {
        app_dirs.push(dirs.pictures.clone());
    }
    if fs.music {
        app_dirs.push(dirs.music.clone());
    }
    if fs.videos {
        app_dirs.push(dirs.videos.clone());
    }
    // `custom` paths are added verbatim, EXCEPT a host-filesystem escape (`/`,
    // an ancestor of the home, or an OS root): Arlen does not offer the
    // `--filesystem=host` grant, so such an entry is dropped, never bound.
    app_dirs.extend(
        fs.custom
            .iter()
            .filter(|p| !is_host_escape(p, home))
            .cloned(),
    );
    ConfinementInputs {
        app_dirs,
        network: network_policy(net),
    }
}

/// Map `[network]` to a [`NetworkPolicy`]. An explicit `allowed_domains` set is the
/// filtered case (the launcher installs the host filter); `allow_all` (with no
/// domain list) is unrestricted egress; neither declared is no network at all.
///
/// `allowed_domains` takes precedence over `allow_all`: an explicit allowlist is
/// the narrower, safer reading of a contradictory profile.
pub fn network_policy(net: &NetworkPermissions) -> NetworkPolicy {
    if !net.allowed_domains.is_empty() {
        NetworkPolicy::FilteredHosts(net.allowed_domains.clone())
    } else if net.allow_all {
        NetworkPolicy::Unrestricted
    } else {
        NetworkPolicy::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs() -> UserDirs {
        UserDirs {
            documents: PathBuf::from("/home/u/Documents"),
            downloads: PathBuf::from("/home/u/Downloads"),
            pictures: PathBuf::from("/home/u/Pictures"),
            music: PathBuf::from("/home/u/Music"),
            videos: PathBuf::from("/home/u/Videos"),
        }
    }

    fn inputs(fs: FilesystemPermissions, net: NetworkPermissions) -> ConfinementInputs {
        confinement_inputs(&fs, &net, "com.example.app", Path::new("/home/u"), &dirs())
    }

    #[test]
    fn always_grants_the_apps_own_state_dirs() {
        let c = inputs(FilesystemPermissions::default(), NetworkPermissions::default());
        assert!(c
            .app_dirs
            .contains(&PathBuf::from("/home/u/.local/share/arlen/apps/com.example.app")));
        assert!(c
            .app_dirs
            .contains(&PathBuf::from("/home/u/.config/arlen/apps/com.example.app")));
    }

    #[test]
    fn a_host_filesystem_custom_grant_is_dropped() {
        // The classic --filesystem=host escapes: the root, the whole /home tree,
        // and the OS roots. None may be bound into a confined app.
        for escape in ["/", "/home", "/etc", "/usr", "/var", "/proc", "/dev", "/home/u"] {
            let fs = FilesystemPermissions {
                custom: vec![PathBuf::from(escape)],
                ..Default::default()
            };
            let c = inputs(fs, NetworkPermissions::default());
            assert!(
                !c.app_dirs.contains(&PathBuf::from(escape)),
                "host-escape custom grant {escape} must be dropped"
            );
        }
    }

    #[test]
    fn a_specific_custom_subdirectory_is_kept() {
        // A real, narrow custom path (a project dir, a data mount) is legitimate
        // and must still be bound - the ban targets only the whole-tree roots.
        let fs = FilesystemPermissions {
            custom: vec![
                PathBuf::from("/home/u/Projects"),
                PathBuf::from("/mnt/data"),
            ],
            ..Default::default()
        };
        let c = inputs(fs, NetworkPermissions::default());
        assert!(c.app_dirs.contains(&PathBuf::from("/home/u/Projects")));
        assert!(c.app_dirs.contains(&PathBuf::from("/mnt/data")));
    }

    #[test]
    fn the_documents_flag_adds_the_documents_dir() {
        let fs = FilesystemPermissions {
            documents: true,
            ..Default::default()
        };
        let c = inputs(fs, NetworkPermissions::default());
        assert!(c.app_dirs.contains(&PathBuf::from("/home/u/Documents")));
        assert!(!c.app_dirs.contains(&PathBuf::from("/home/u/Downloads")));
    }

    #[test]
    fn the_home_flag_adds_the_home_dir() {
        let fs = FilesystemPermissions {
            home: true,
            ..Default::default()
        };
        let c = inputs(fs, NetworkPermissions::default());
        assert!(c.app_dirs.contains(&PathBuf::from("/home/u")));
    }

    #[test]
    fn custom_paths_are_added_verbatim() {
        let fs = FilesystemPermissions {
            custom: vec![PathBuf::from("/opt/shared/data")],
            ..Default::default()
        };
        let c = inputs(fs, NetworkPermissions::default());
        assert!(c.app_dirs.contains(&PathBuf::from("/opt/shared/data")));
    }

    #[test]
    fn allowed_domains_map_to_filtered_hosts() {
        let net = NetworkPermissions {
            allowed_domains: vec!["api.example.org:443".into()],
            ..Default::default()
        };
        assert_eq!(
            network_policy(&net),
            NetworkPolicy::FilteredHosts(vec!["api.example.org:443".into()])
        );
    }

    #[test]
    fn allow_all_maps_to_unrestricted() {
        let net = NetworkPermissions {
            allow_all: true,
            ..Default::default()
        };
        assert_eq!(network_policy(&net), NetworkPolicy::Unrestricted);
    }

    #[test]
    fn no_network_declared_maps_to_none() {
        assert_eq!(network_policy(&NetworkPermissions::default()), NetworkPolicy::None);
    }

    #[test]
    fn an_allowlist_takes_precedence_over_allow_all() {
        let net = NetworkPermissions {
            allow_all: true,
            allowed_domains: vec!["api.example.org:443".into()],
        };
        assert!(matches!(network_policy(&net), NetworkPolicy::FilteredHosts(_)));
    }
}
