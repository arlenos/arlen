//! Mapping a permission profile to the confiner's inputs.
//!
//! `arlen-run` reads an app's [`PermissionProfile`] and derives the writable
//! filesystem set and the network policy the confiner needs: the app's own state
//! dirs are always writable, the `[filesystem]` flags add the matching XDG user
//! dirs, `custom` paths are added verbatim, and `[network]` maps to a
//! [`NetworkPolicy`]. The result feeds `arlen_confiner::app_runtime_profile`.

use std::path::{Path, PathBuf};

use arlen_confiner::NetworkPolicy;
use arlen_permissions::{expand_user, FilesystemPermissions, NetworkPermissions};

// `UserDirs`, `is_host_escape` and `read_only_grant_ok` moved to
// `arlen_permissions` beside `FilesystemPermissions`, and the flag-to-directory
// mapping with them. The launcher is no longer the only component that has to
// answer what a grant reaches - the launch service needs the same answer to gate
// its mime query - and two readings of one grant is how an app ends up confined
// to one set and told about another.
pub use arlen_permissions::{is_host_escape, read_only_grant_ok, UserDirs};

/// The confiner inputs derived from a profile: the read-write set and the network
/// policy. `arlen-run` passes these to `app_runtime_profile`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfinementInputs {
    /// Directories the app may write (its own state dirs, the flag-gated XDG dirs,
    /// and any `custom` paths). Each becomes a read-write bind.
    pub app_dirs: Vec<PathBuf>,
    /// Directories masked from the app even under a broader grant (a tmpfs over
    /// each), so e.g. a `home` grant cannot expose `~/.config/arlen` (the system
    /// AI/shell/compositor configs + other apps' configs). The app's own state
    /// dirs that lie under a masked path are re-exposed after the mask (the
    /// confiner's `post_mask_binds`). (same-uid-isolation-plan.md Tier-A #3.)
    pub masked_dirs: Vec<PathBuf>,
    /// Subtrees bound READ-ONLY. Separate from `app_dirs` because the difference
    /// is the whole point of the grant: an app that needs to read `/sys` could
    /// otherwise only ask for a writable bind, which the forbidden-roots rule
    /// drops - so the narrow thing was unsayable and the wide thing was refused.
    pub read_only_dirs: Vec<PathBuf>,
    /// The network policy.
    pub network: NetworkPolicy,
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
    // The flag-gated dirs and the accepted `custom` paths, from the one place
    // that reads the grant. WRITABLE only: `read_only` subtrees are collected
    // separately below, and folding them in here is what the read-only test
    // below catches.
    // A RELATIVE `custom` PATH IS DROPPED, not resolved. `read_only_grant_ok`
    // already refuses one and says why - it would resolve against the LAUNCHER's
    // cwd, which has nothing to do with the app - and `custom` had no such rule,
    // so a path the grammar could not expand went to bwrap verbatim. The calendar
    // wrote `$HOME/.local/share/arlen/calendars`, and `$HOME` is not a token
    // `expand_user` knows, so the grant stayed relative and meant nothing.
    app_dirs.extend(
        fs.writable_dirs(home, dirs)
            .into_iter()
            .filter(|p| p.is_absolute()),
    );

    // Read-only subtrees: the same whole-tree refusal, and a named subtree under
    // one of those roots is allowed - that is what makes `/sys/class/power_supply`
    // sayable without making `/sys` bindable.
    // EXPANDED FIRST, like `custom`, then gated. A profile is written once for
    // whoever installs it, so every shipped one spells the home tree
    // `/home/$USER`; cloned verbatim that is a path no machine has, and the
    // launcher drops a bind whose source does not exist. The reader's entire
    // filesystem grant resolved to nothing and it could not open the document it
    // was handed. The gate runs on the expanded path so its depth rule measures
    // the directory that will actually be bound.
    let read_only_dirs: Vec<PathBuf> = fs
        .read_only
        .iter()
        .map(|p| expand_user(p, home))
        .filter(|p| read_only_grant_ok(p, home))
        .collect();
    ConfinementInputs {
        app_dirs,
        read_only_dirs,
        // Always mask the system arlen config dir: only the app's own
        // `~/.config/arlen/apps/{app_id}` (in `app_dirs`) is re-exposed, so a
        // broad grant (`home`, or a `custom` ancestor of `~/.config`) cannot
        // hand a confined app the AI master switches' file siblings, the shell /
        // compositor config, or other apps' configs. Harmless when no grant
        // would have exposed it (a tmpfs over an otherwise-absent path).
        //
        // And the user's systemd unit directory, which is a WRITE that becomes an
        // identity. `systemd-analyze --user unit-paths` ranks it above
        // `/usr/lib/systemd/user`, so a file dropped there overrides a unit we
        // ship: an app that can write it can define `arlen-knowledge.service` to
        // run its own binary, have systemd start it, and be that daemon to
        // everything resolving identity in the user session. Measured 13 Aug -
        // a hand-written unit runs any binary under any name, no privilege
        // needed. Without this mask every user-session resolver above it is
        // decoration.
        masked_dirs: vec![home.join(".config/arlen"), home.join(".config/systemd/user")],
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

    pub(super) fn dirs() -> UserDirs {
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
    fn the_masks_hold_under_the_broadest_grant_that_would_expose_them() {
        // Both masks are unconditional, but the case they exist for is a `home`
        // grant, which otherwise binds the whole home writable - so assert them
        // there rather than on a default profile that grants nothing.
        let fs = FilesystemPermissions {
            home: true,
            ..FilesystemPermissions::default()
        };
        let c = inputs(fs, NetworkPermissions::default());
        assert!(
            c.masked_dirs.contains(&PathBuf::from("/home/u/.config/arlen")),
            "the system config dir must stay masked: {:?}",
            c.masked_dirs
        );
        // A write here is an identity, not a setting: the directory outranks
        // `/usr/lib/systemd/user`, so a unit dropped in it overrides one we ship
        // and the app becomes that daemon to every user-session resolver.
        assert!(
            c.masked_dirs.contains(&PathBuf::from("/home/u/.config/systemd/user")),
            "the user unit dir must stay masked: {:?}",
            c.masked_dirs
        );
        // And the home grant really is what would otherwise expose them.
        assert!(c.app_dirs.contains(&PathBuf::from("/home/u")), "{:?}", c.app_dirs);
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

#[cfg(test)]
mod read_only_grant {
    use super::tests::dirs;
    use super::*;

    #[test]
    fn a_named_subtree_under_a_forbidden_root_is_grantable_read_only() {
        let home = Path::new("/home/u");
        // The case that motivated the grant: the system monitor needs these and
        // could not say so, because the only thing the format offered was a
        // writable `custom` that the forbidden-roots rule drops.
        assert!(read_only_grant_ok(Path::new("/sys/class/power_supply"), home));
        assert!(read_only_grant_ok(Path::new("/sys/devices/system/cpu"), home));
    }

    #[test]
    fn the_whole_tree_roots_stay_refused_even_read_only() {
        let home = Path::new("/home/u");
        for root in ["/sys", "/etc", "/usr", "/", "/proc", "/dev"] {
            assert!(
                !read_only_grant_ok(Path::new(root), home),
                "{root} read-only is still the shape the rule exists to stop"
            );
        }
        // And the home's ancestors, for the same reason `custom` refuses them.
        assert!(!read_only_grant_ok(Path::new("/home"), home));
    }

    #[test]
    fn a_relative_path_is_refused_rather_than_resolved() {
        // It would resolve against the LAUNCHER's cwd, which has nothing to do
        // with the app - so it is refused rather than quietly made absolute.
        assert!(!read_only_grant_ok(Path::new("sys/class"), Path::new("/home/u")));
    }

    #[test]
    fn a_read_only_grant_never_lands_in_the_writable_set() {
        let fs = FilesystemPermissions {
            read_only: vec![PathBuf::from("/sys/class/power_supply")],
            ..Default::default()
        };
        let c = confinement_inputs(
            &fs,
            &NetworkPermissions::default(),
            "com.example.app",
            Path::new("/home/u"),
            &dirs(),
        );
        assert!(c.read_only_dirs.contains(&PathBuf::from("/sys/class/power_supply")));
        assert!(
            !c.app_dirs.contains(&PathBuf::from("/sys/class/power_supply")),
            "a read-only grant must not become a writable bind"
        );
    }

    #[test]
    fn a_custom_grant_the_grammar_cannot_expand_is_dropped_not_bound() {
        // `$HOME` is not a token `expand_user` knows - only `$USER` is - so this
        // stays relative, and a relative bind source resolves against whatever
        // the launcher's cwd happens to be. Refused, the way the read-only gate
        // already refuses one.
        let fs = FilesystemPermissions {
            custom: vec![PathBuf::from("$HOME/.local/share/arlen/calendars")],
            ..Default::default()
        };
        let c = confinement_inputs(
            &fs,
            &NetworkPermissions::default(),
            "com.example.app",
            Path::new("/home/u"),
            &dirs(),
        );
        assert!(
            !c.app_dirs
                .iter()
                .any(|p| p.to_string_lossy().contains("$HOME")),
            "got {:?}",
            c.app_dirs
        );
    }

    #[test]
    fn a_read_only_grant_gets_the_same_user_expansion_custom_does() {
        // Every shipped profile spells a per-user path `/home/$USER` or
        // `/run/media/$USER`, because a profile is written once for whoever
        // installs it. `custom` maps through `expand_user`; `read_only` was
        // cloned verbatim, so the grant resolved to a path no machine has and
        // the launcher dropped the bind for a source that does not exist.
        let fs = FilesystemPermissions {
            read_only: vec![PathBuf::from("/run/media/$USER")],
            ..Default::default()
        };
        let c = confinement_inputs(
            &fs,
            &NetworkPermissions::default(),
            "com.example.app",
            Path::new("/home/u"),
            &dirs(),
        );
        assert_eq!(c.read_only_dirs, vec![PathBuf::from("/run/media/u")]);
    }

    #[test]
    fn the_home_tree_is_refused_read_only_like_any_other_whole_tree() {
        // Not a gap in the expansion above: `is_host_escape` refuses any path
        // home starts with, so `/home/$USER` is dropped after expanding too. It
        // is the deliberate whole-tree rule, and it means an app cannot ask to
        // READ the home tree without writing it - the reader's profile asks for
        // exactly that and gets nothing.
        let fs = FilesystemPermissions {
            read_only: vec![PathBuf::from("/home/$USER")],
            ..Default::default()
        };
        let c = confinement_inputs(
            &fs,
            &NetworkPermissions::default(),
            "com.example.app",
            Path::new("/home/u"),
            &dirs(),
        );
        assert!(c.read_only_dirs.is_empty(), "got {:?}", c.read_only_dirs);
    }

    #[test]
    fn a_forbidden_root_asked_for_read_only_is_dropped_not_bound() {
        let fs = FilesystemPermissions {
            read_only: vec![PathBuf::from("/etc"), PathBuf::from("/sys")],
            ..Default::default()
        };
        let c = confinement_inputs(
            &fs,
            &NetworkPermissions::default(),
            "com.example.app",
            Path::new("/home/u"),
            &dirs(),
        );
        assert!(c.read_only_dirs.is_empty(), "{:?}", c.read_only_dirs);
    }
}
