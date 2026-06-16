//! Shared unprivileged confiner for Arlen, built on bubblewrap (`bwrap`).
//!
//! One base confinement with two profiles, so the security primitive is built
//! once and reused (roadmap D1 / F-R2, `package-capability-enrollment.md` §4):
//!
//! - [`build_profile`]: the forage build sandbox (`forage-recipes.md` §10/§10a)
//!   - no network, the pinned base platform as a read-only `/`, the build
//!     directory writable at a fixed `/build`, a tmpfs `/tmp` and `/sys`, a
//!     deterministic environment with a fixed PATH. Returns a runnable
//!     [`Confinement`].
//! - [`app_runtime_profile`]: the `arlen-run` confiner for installed apps,
//!   returned as an [`AppProfileSkeleton`] that is **not directly runnable** —
//!   the launcher must [`AppProfileSkeleton::complete`] it with the universal
//!   plumbing, then apply Landlock and the network host-filter, before spawning.
//!
//! This crate constructs the `bwrap` argument vector ([`Confinement::bwrap_args`],
//! pure and unit-tested). Layers applied by the caller at launch on a real
//! kernel, not here: the build-appropriate **seccomp** allowlist
//! (`bwrap --seccomp <fd>`), **Landlock** (app profile), the **network filter**
//! (app profile), and **reaping** the process tree (a pid-namespace + cgroup
//! kill-all; `bwrap --die-with-parent` handles the parent-death case only).
//! `bwrap` itself sets `no_new_privs` and the namespaces.

use std::collections::BTreeMap;
use std::path::Path;

use thiserror::Error;

/// The fixed in-sandbox path the build directory is mounted at, so the build
/// never sees the host path (determinism, `-ffile-prefix-map`).
pub const BUILD_MOUNT: &str = "/build";

/// Network confinement for a profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkPolicy {
    /// No network at all (`--unshare-net`). The build profile uses this.
    None,
    /// Network is available but the allowed-host set is enforced by a separate
    /// filter the launcher installs (not by bwrap). The app profile uses this.
    FilteredHosts(Vec<String>),
    /// Network is available with NO host filter (an app that declared `allow_all`).
    /// Like [`NetworkPolicy::FilteredHosts`] bwrap leaves the network up (no
    /// `--unshare-net`); unlike it the launcher installs no egress filter. The
    /// widest egress posture, for apps that explicitly request unrestricted network.
    Unrestricted,
}

/// A read-only or read-write bind mount from host to the confined view. Both
/// the source (host) and destination (in-sandbox) paths are validated absolute,
/// UTF-8 strings at construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bind {
    /// Read-only bind: `--ro-bind src dest`.
    ReadOnly(String, String),
    /// Read-write bind: `--bind src dest`.
    ReadWrite(String, String),
}

/// A runnable confinement spec. The namespaces are always applied; binds,
/// tmpfs dirs, environment and the network policy are profile-specific.
#[derive(Debug, Clone)]
pub struct Confinement {
    network: NetworkPolicy,
    binds: Vec<Bind>,
    tmpfs: Vec<String>,
    env: BTreeMap<String, String>,
    chdir: Option<String>,
}

/// A failure building a confinement.
#[derive(Debug, Error)]
pub enum ConfinerError {
    /// A path was not absolute (bwrap requires absolute paths; a relative host
    /// path is ambiguous).
    #[error("path must be absolute: {0}")]
    RelativePath(String),
    /// A path was not valid UTF-8 (it would be silently mangled into the argv).
    #[error("path is not valid UTF-8: {0}")]
    NonUtf8Path(String),
    /// The build directory overlaps the read-only base platform root.
    #[error("build dir {build} overlaps the base platform {base}")]
    OverlappingPaths {
        /// The build directory.
        build: String,
        /// The base platform root.
        base: String,
    },
    /// The caller supplied an environment variable the build profile reserves.
    #[error("environment variable `{0}` is reserved by the build profile")]
    ReservedEnv(String),
}

impl Confinement {
    /// Render the `bwrap` argument vector, **excluding** the program and its
    /// arguments (the caller appends `-- <program> <args>`) and `--seccomp <fd>`
    /// (attached at spawn). Deterministic for a given spec.
    ///
    /// Ordering is load-bearing: bwrap applies filesystem operations in argv
    /// order against the new root, so the binds (the read-only root first) are
    /// emitted **before** `--proc`/`--dev`/`--tmpfs`, otherwise mounting the
    /// base platform over `/` would shadow the private procfs/devtmpfs/tmpfs.
    pub fn bwrap_args(&self) -> Vec<String> {
        let mut a: Vec<String> = Vec::new();
        for flag in [
            "--unshare-user",
            "--unshare-pid",
            "--unshare-ipc",
            "--unshare-uts",
            "--unshare-cgroup-try",
            "--new-session",
            "--die-with-parent",
            "--clearenv",
        ] {
            a.push(flag.into());
        }
        if self.network == NetworkPolicy::None {
            a.push("--unshare-net".into());
        }
        // Binds first (the spec puts the read-only root first), so the root is
        // established before anything is layered onto it.
        for bind in &self.binds {
            match bind {
                Bind::ReadOnly(src, dst) => {
                    a.push("--ro-bind".into());
                    a.push(src.clone());
                    a.push(dst.clone());
                }
                Bind::ReadWrite(src, dst) => {
                    a.push("--bind".into());
                    a.push(src.clone());
                    a.push(dst.clone());
                }
            }
        }
        // Private procfs and minimal devtmpfs, layered on top of the root bind.
        a.push("--proc".into());
        a.push("/proc".into());
        a.push("--dev".into());
        a.push("/dev".into());
        for t in &self.tmpfs {
            a.push("--tmpfs".into());
            a.push(t.clone());
        }
        // Deterministic, sorted environment (BTreeMap iterates sorted).
        for (k, v) in &self.env {
            a.push("--setenv".into());
            a.push(k.clone());
            a.push(v.clone());
        }
        if let Some(dir) = &self.chdir {
            a.push("--chdir".into());
            a.push(dir.clone());
        }
        a
    }

    /// The network policy this confinement declares (the launcher installs the
    /// host filter for [`NetworkPolicy::FilteredHosts`]).
    pub fn network(&self) -> &NetworkPolicy {
        &self.network
    }

    /// Override the working directory (an in-sandbox absolute path). The build
    /// profile defaults to [`BUILD_MOUNT`]; a per-step caller sets a sub-path
    /// under it (e.g. for a build step's `workdir`).
    pub fn with_chdir(mut self, dir: impl Into<String>) -> Self {
        self.chdir = Some(dir.into());
        self
    }
}

/// Environment keys the build profile owns; a recipe-supplied value for any of
/// these is rejected, because they steer execution and would defeat the pinned,
/// deterministic toolchain (PATH) or inject libraries (the loader family).
fn is_reserved_build_env(key: &str) -> bool {
    key == "PATH" || key.starts_with("LD_")
}

/// The forage build sandbox profile (`forage-recipes.md` §10a): no network, the
/// pinned base platform as a read-only `/`, the build directory writable at the
/// fixed [`BUILD_MOUNT`], tmpfs `/tmp` and `/sys`, a fixed PATH, and the given
/// deterministic environment.
///
/// `env` must not set [`is_reserved_build_env`] keys (PATH or `LD_*`); the
/// profile sets PATH itself. `build_dir` must not overlap `base_platform`.
pub fn build_profile(
    base_platform: &Path,
    build_dir: &Path,
    env: BTreeMap<String, String>,
) -> Result<Confinement, ConfinerError> {
    let base = checked_abs(base_platform)?;
    let build = checked_abs(build_dir)?;

    // The build dir must not live inside the read-only platform root (a writable
    // window into the immutable toolchain), nor contain it.
    if path_overlaps(&build, &base) {
        return Err(ConfinerError::OverlappingPaths { build, base });
    }

    // The profile owns PATH and the loader env; reject caller attempts to set
    // them, then install a fixed PATH into the platform's tools.
    for key in env.keys() {
        if is_reserved_build_env(key) {
            return Err(ConfinerError::ReservedEnv(key.clone()));
        }
    }
    let mut env = env;
    env.insert("PATH".into(), "/usr/bin:/bin".into());

    Ok(Confinement {
        network: NetworkPolicy::None,
        binds: vec![
            // The toolchain (base platform) is the read-only root, emitted first.
            Bind::ReadOnly(base, BUILD_ROOT.into()),
            // The build directory is writable at the fixed in-sandbox path.
            Bind::ReadWrite(build, BUILD_MOUNT.into()),
        ],
        tmpfs: vec!["/tmp".into(), "/sys".into()],
        env,
        chdir: Some(BUILD_MOUNT.into()),
    })
}

const BUILD_ROOT: &str = "/";

/// An app-runtime confinement that is **not yet runnable**: it has the shared
/// base plus the app's security-axis binds (`/usr` read-only, the app's own
/// state dirs writable) and the network policy, but not the universal plumbing
/// (Wayland/PipeWire/D-Bus/fonts) or Landlock. It has no `bwrap_args`; the
/// launcher must [`complete`](AppProfileSkeleton::complete) it with the plumbing
/// binds (and separately apply Landlock + the network filter) before spawning,
/// so a bare skeleton can never be run under-confined by mistake.
#[derive(Debug, Clone)]
pub struct AppProfileSkeleton {
    inner: Confinement,
}

impl AppProfileSkeleton {
    /// The declared network policy (the launcher installs the host filter).
    pub fn network(&self) -> &NetworkPolicy {
        self.inner.network()
    }

    /// The security-axis binds the skeleton carries (for the launcher to
    /// inspect/extend).
    pub fn binds(&self) -> &[Bind] {
        &self.inner.binds
    }

    /// Complete the skeleton into a runnable [`Confinement`] by adding the
    /// universal-plumbing binds and tmpfs the launcher determined for this app
    /// (the Wayland socket, PipeWire, a filtered session D-Bus, fonts, ...).
    /// The launcher still applies Landlock and the network host-filter on top.
    pub fn complete(mut self, plumbing: Vec<Bind>, tmpfs: Vec<String>) -> Confinement {
        self.inner.binds.extend(plumbing);
        self.inner.tmpfs.extend(tmpfs);
        self.inner
    }
}

/// The app-runtime confiner profile skeleton (`package-capability-enrollment.md`
/// §4): read-only `/usr`, the app's own state dirs writable, the network policy.
/// See [`AppProfileSkeleton`] — it is deliberately not directly runnable.
pub fn app_runtime_profile(
    usr: &Path,
    app_dirs: &[&Path],
    env: BTreeMap<String, String>,
    net: NetworkPolicy,
) -> Result<AppProfileSkeleton, ConfinerError> {
    let usr = checked_abs(usr)?;
    let mut binds = vec![Bind::ReadOnly(usr, "/usr".into())];
    for dir in app_dirs {
        let d = checked_abs(dir)?;
        binds.push(Bind::ReadWrite(d.clone(), d));
    }
    Ok(AppProfileSkeleton {
        inner: Confinement {
            network: net,
            binds,
            tmpfs: vec!["/tmp".into()],
            env,
            chdir: None,
        },
    })
}

/// Validate a path is absolute and UTF-8, returning it as a string. Rejects
/// non-UTF8 (which `to_string_lossy` would silently corrupt in the argv).
fn checked_abs(p: &Path) -> Result<String, ConfinerError> {
    let s = p
        .to_str()
        .ok_or_else(|| ConfinerError::NonUtf8Path(p.to_string_lossy().into_owned()))?;
    if !p.is_absolute() {
        return Err(ConfinerError::RelativePath(s.to_string()));
    }
    Ok(s.to_string())
}

/// Whether two absolute paths overlap (one is a path-component prefix of the
/// other), using a lexical check on the already-validated absolute strings.
fn path_overlaps(a: &str, b: &str) -> bool {
    let pa = Path::new(a);
    let pb = Path::new(b);
    pa.starts_with(pb) || pb.starts_with(pa)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn env() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("SOURCE_DATE_EPOCH".to_string(), "0".to_string()),
            ("LC_ALL".to_string(), "C".to_string()),
        ])
    }

    fn after<'a>(args: &'a [String], flag: &str) -> Vec<&'a str> {
        args.iter()
            .enumerate()
            .filter(|(_, a)| a.as_str() == flag)
            .map(|(i, _)| args[i + 1].as_str())
            .collect()
    }

    fn index_of(args: &[String], flag: &str) -> usize {
        args.iter().position(|a| a == flag).expect("flag present")
    }

    #[test]
    fn build_profile_confinement_and_fixed_build_path() {
        let conf = build_profile(
            Path::new("/var/lib/arlen/platform/2026"),
            Path::new("/var/tmp/forage/work"),
            env(),
        )
        .unwrap();
        let args = conf.bwrap_args();
        assert!(args.contains(&"--unshare-net".to_string()));
        assert!(args.contains(&"--clearenv".to_string()));
        // Build dir is mounted at the fixed /build, and is the working dir.
        assert!(rw_pairs(&args).contains(&("/var/tmp/forage/work".into(), "/build".into())));
        assert_eq!(after(&args, "--chdir"), vec!["/build"]);
        // tmpfs covers /tmp and /sys.
        let mut tmpfs = after(&args, "--tmpfs");
        tmpfs.sort();
        assert_eq!(tmpfs, vec!["/sys", "/tmp"]);
        // PATH is set by the profile.
        assert!(args.windows(2).any(|w| w[0] == "PATH" && w[1] == "/usr/bin:/bin"));
    }

    #[test]
    fn root_bind_precedes_proc_and_dev() {
        // H1: the / bind must come before --proc/--dev or it shadows them.
        let conf = build_profile(Path::new("/platform"), Path::new("/work"), env()).unwrap();
        let args = conf.bwrap_args();
        let root_bind = args.windows(3).position(|w| w[0] == "--ro-bind" && w[2] == "/").unwrap();
        assert!(root_bind < index_of(&args, "--proc"), "/ bind before --proc");
        assert!(root_bind < index_of(&args, "--dev"), "/ bind before --dev");
        assert!(root_bind < index_of(&args, "--tmpfs"), "/ bind before --tmpfs");
    }

    #[test]
    fn build_dir_overlapping_base_is_rejected() {
        // H2: a build dir inside the base platform would punch a writable hole.
        assert!(matches!(
            build_profile(Path::new("/platform"), Path::new("/platform/work"), env()),
            Err(ConfinerError::OverlappingPaths { .. })
        ));
        assert!(matches!(
            build_profile(Path::new("/platform/sub"), Path::new("/platform"), env()),
            Err(ConfinerError::OverlappingPaths { .. })
        ));
    }

    #[test]
    fn build_profile_rejects_reserved_env() {
        // M3: PATH and LD_* are owned by the profile, not the recipe.
        for key in ["PATH", "LD_PRELOAD", "LD_LIBRARY_PATH", "LD_AUDIT"] {
            let mut e = env();
            e.insert(key.into(), "/evil".into());
            assert!(
                matches!(
                    build_profile(Path::new("/p"), Path::new("/w"), e),
                    Err(ConfinerError::ReservedEnv(_))
                ),
                "`{key}` must be reserved"
            );
        }
    }

    #[test]
    fn rejects_relative_and_non_utf8_paths() {
        assert!(matches!(
            build_profile(Path::new("rel"), Path::new("/w"), env()),
            Err(ConfinerError::RelativePath(_))
        ));
        #[cfg(unix)]
        {
            use std::ffi::OsStr;
            use std::os::unix::ffi::OsStrExt;
            let bad = PathBuf::from(OsStr::from_bytes(b"/\xff\xfe"));
            assert!(matches!(
                build_profile(&bad, Path::new("/w"), env()),
                Err(ConfinerError::NonUtf8Path(_))
            ));
        }
    }

    #[test]
    fn with_chdir_overrides_the_working_directory() {
        let conf = build_profile(Path::new("/p"), Path::new("/w"), env())
            .unwrap()
            .with_chdir("/build/sub");
        assert_eq!(after(&conf.bwrap_args(), "--chdir"), vec!["/build/sub"]);
    }

    #[test]
    fn determinism() {
        let a = build_profile(Path::new("/p"), Path::new("/w"), env()).unwrap();
        let b = build_profile(Path::new("/p"), Path::new("/w"), env()).unwrap();
        assert_eq!(a.bwrap_args(), b.bwrap_args());
    }

    #[test]
    fn app_skeleton_is_not_runnable_until_completed() {
        // L1: the skeleton has no bwrap_args; only a completed Confinement does.
        let skel = app_runtime_profile(
            Path::new("/usr"),
            &[Path::new("/home/u/.config/demo")],
            env(),
            NetworkPolicy::FilteredHosts(vec!["api.example.org:443".into()]),
        )
        .unwrap();
        assert!(matches!(skel.network(), NetworkPolicy::FilteredHosts(_)));
        assert!(skel
            .binds()
            .contains(&Bind::ReadOnly("/usr".into(), "/usr".into())));
        // Completing it (as arlen-run would) yields a runnable Confinement that
        // keeps the network policy (no --unshare-net).
        let conf = skel.complete(
            vec![Bind::ReadWrite("/run/user/1000/wayland-0".into(), "/run/user/1000/wayland-0".into())],
            vec![],
        );
        let args = conf.bwrap_args();
        assert!(!args.contains(&"--unshare-net".to_string()));
        assert!(rw_pairs(&args)
            .contains(&("/home/u/.config/demo".into(), "/home/u/.config/demo".into())));
    }

    #[test]
    fn unrestricted_network_leaves_the_network_up() {
        // An `allow_all` app maps to Unrestricted: network up (no --unshare-net),
        // and the launcher installs no filter (FilteredHosts is the filtered case).
        let skel = app_runtime_profile(
            Path::new("/usr"),
            &[],
            BTreeMap::new(),
            NetworkPolicy::Unrestricted,
        )
        .unwrap();
        assert!(matches!(skel.network(), NetworkPolicy::Unrestricted));
        let args = skel.complete(vec![], vec![]).bwrap_args();
        assert!(!args.contains(&"--unshare-net".to_string()));
    }

    fn rw_pairs(args: &[String]) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < args.len() {
            if args[i] == "--bind" {
                out.push((args[i + 1].clone(), args[i + 2].clone()));
                i += 3;
            } else {
                i += 1;
            }
        }
        out
    }

    #[test]
    fn path_overlaps_is_component_wise_not_substring() {
        // Identical, parent, and child all overlap (both directions).
        assert!(path_overlaps("/a/b", "/a/b"));
        assert!(path_overlaps("/a/b/c", "/a/b"));
        assert!(path_overlaps("/a/b", "/a/b/c"));
        // Root contains everything.
        assert!(path_overlaps("/", "/anything/deep"));
        // Siblings do not overlap.
        assert!(!path_overlaps("/a/b", "/a/c"));
        // The security-critical case: a shared partial component is NOT an overlap
        // (a lexical str::starts_with would wrongly flag these).
        assert!(!path_overlaps("/base", "/basement"));
        assert!(!path_overlaps("/srv/buildroot", "/srv/build"));
    }

    #[test]
    fn is_reserved_build_env_matches_path_and_ld_prefix_exactly() {
        assert!(is_reserved_build_env("PATH"));
        assert!(is_reserved_build_env("LD_PRELOAD"));
        assert!(is_reserved_build_env("LD_LIBRARY_PATH"));
        assert!(is_reserved_build_env("LD_"));
        // Not reserved: a longer name sharing the PATH prefix, the wrong case, or
        // LD_ embedded rather than leading.
        assert!(!is_reserved_build_env("PATHEXT"));
        assert!(!is_reserved_build_env("path"));
        assert!(!is_reserved_build_env("MY_LD_HACK"));
        assert!(!is_reserved_build_env("HOME"));
    }
}
