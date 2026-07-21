//! OA-R3: run `rclone` confined under `arlen-run`.
//!
//! The mount backend is a network-touching subprocess, so it runs as its own
//! process under `arlen-run` (Landlock + seccomp + a per-launch cgroup + the
//! egress netns from `arlen-run`'s §0 enforcer), NOT linked into this audited
//! daemon. `arlen-run` refuses a `FilteredHosts` launch until §0 lands; it does
//! now, so this launcher builds the invocation.
//!
//! The daemon writes a per-mount permission profile (its `[network]
//! allowed_domains` = [`egress_hosts`], its `[filesystem]` = the runtime socket
//! dir + the mount point + rclone's config) to a profile-root dir, then spawns
//! [`confined_rclone_argv`]. rclone serves its rc API on the AF_UNIX socket the
//! daemon's `RcClient` drives; the socket is a filesystem path (unaffected by the
//! egress netns), reachable only by the daemon (the trust boundary).
//!
//! The `Files.Mount`/`Unmount` D-Bus methods (dbus.rs) drive these: `resolve_mount`
//! then `spawn_confined_mount`, and `RcloneMount::unmount` to tear down.

use std::io;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use tokio::process::{Child, Command};

use crate::config::{AccountConfig, FilesBackend};
use crate::connection::ConnectionBackend;
use crate::mount::{plan_mount, MountError, MountPlan};
use crate::rc::{RcClient, RcError, UnixRcTransport};

/// How long to wait for the confined rclone to open its rc socket before giving
/// up (fail-closed: a launch that never serves is not silently left running).
const SOCKET_TIMEOUT: Duration = Duration::from_secs(10);

/// The reverse-DNS app id the confined rclone runs under; `arlen-run` resolves
/// its profile at `<profile-root>/{RCLONE_APP_ID}.toml`. One mount identity for
/// the accounts daemon's rclone subprocess.
pub const RCLONE_APP_ID: &str = "org.arlen.accounts.rclone";

/// The `host:port` set the mount is permitted to egress to (its provider), for
/// the `FilteredHosts` profile `arlen-run` enforces. Empty when the backend
/// declares no dialable host - the caller must then refuse the mount, fail-closed
/// (an empty allowlist would confine rclone to nothing, but a mount with no host
/// is a misconfiguration, not a valid zero-egress request).
pub fn egress_hosts(backend: &FilesBackend) -> Vec<String> {
    let host_port = match backend.backend {
        ConnectionBackend::Sftp => host_with_port(backend.host.as_deref(), backend.port, 22),
        ConnectionBackend::Ftp => host_with_port(backend.host.as_deref(), backend.port, 21),
        ConnectionBackend::Webdav => webdav_host_port(backend.url.as_deref()),
    };
    host_port.into_iter().collect()
}

/// `host:port` for the direct-host backends, defaulting the port when the config
/// omits it (rclone would apply the same default). `None` on an empty/absent host.
fn host_with_port(host: Option<&str>, port: Option<u16>, default: u16) -> Option<String> {
    let host = host.map(str::trim).filter(|h| !h.is_empty())?;
    Some(format!("{host}:{}", port.unwrap_or(default)))
}

/// `host:port` from a WebDAV URL (`https://host[:port]/path`), defaulting the
/// port from the scheme (443 https, 80 http). `None` on a malformed/empty URL or
/// an unsupported scheme - the caller then refuses the mount.
fn webdav_host_port(url: Option<&str>) -> Option<String> {
    let url = url.map(str::trim).filter(|u| !u.is_empty())?;
    let (scheme, rest) = url.split_once("://")?;
    let default_port = match scheme.to_ascii_lowercase().as_str() {
        "https" => 443,
        "http" => 80,
        _ => return None,
    };
    // The authority ends at the path/query/fragment; strip any `user@` prefix.
    let authority = rest.split(['/', '?', '#']).next()?;
    let hostport = authority.rsplit('@').next().filter(|h| !h.is_empty())?;
    // An explicit `:port` (with a numeric port) is kept verbatim; otherwise the
    // scheme default is appended. A trailing `:` or a non-numeric port falls to
    // the default (so a stray colon never yields a bogus allowlist entry).
    match hostport.rsplit_once(':') {
        Some((h, p)) if !h.is_empty() && p.parse::<u16>().is_ok() => Some(hostport.to_string()),
        _ => Some(format!("{hostport}:{default_port}")),
    }
}

/// Resolve the `arlen-run` confiner binary: the `ARLEN_RUN_BIN` override (a
/// non-standard install or a test), else the canonical libexec path, else bare
/// `arlen-run` on `PATH`. `arlen-run` has no daemon unit - it is invoked here, so
/// the daemon must locate it.
pub fn arlen_run_binary() -> PathBuf {
    if let Some(p) = std::env::var_os("ARLEN_RUN_BIN") {
        return PathBuf::from(p);
    }
    let libexec = PathBuf::from("/usr/lib/arlen/libexec/arlen-run");
    if libexec.exists() {
        return libexec;
    }
    PathBuf::from("arlen-run")
}

/// The `arlen-run` argv that runs `rclone rcd` confined. `arlen-run` (the
/// `arlen_run` binary) reads the per-mount profile at
/// `<profile_root>/{RCLONE_APP_ID}.toml` (Landlock + seccomp + cgroup + the
/// FilteredHosts egress netns), then execs rclone, which serves its rc API on the
/// AF_UNIX `rc_socket`. `--rc-no-auth` is safe: the socket is a private path in
/// the per-mount runtime dir, reachable only by the daemon. Returned as
/// `[program, args...]`; the caller spawns `program` with `args`.
pub fn confined_rclone_argv(arlen_run: &Path, profile_root: &Path, rc_socket: &Path) -> Vec<String> {
    vec![
        arlen_run.to_string_lossy().into_owned(),
        "--app-id".to_string(),
        RCLONE_APP_ID.to_string(),
        "--profile-root".to_string(),
        profile_root.to_string_lossy().into_owned(),
        "--".to_string(),
        "rclone".to_string(),
        "rcd".to_string(),
        "--rc-no-auth".to_string(),
        format!("--rc-addr=unix://{}", rc_socket.display()),
    ]
}

/// The per-mount filesystem layout for one confined rclone: the runtime dir that
/// holds its profile + rc socket, the socket path, and the read-write set its
/// profile grants. All three writable paths must EXIST before the launch (a
/// missing writable path is dropped from the Landlock grant), so the caller
/// creates them.
pub struct MountPaths {
    /// The per-mount runtime dir (`<runtime_dir>/arlen/accounts/{id}`) holding the
    /// profile and the rc socket; the `--profile-root` arlen-run reads.
    pub profile_root: PathBuf,
    /// The AF_UNIX rc-API socket rclone serves on (inside `profile_root`); the
    /// daemon's `RcClient` drives it.
    pub rc_socket: PathBuf,
    /// The confined rclone's read-write set: its runtime dir (to create the
    /// socket), the mount point (the FUSE target), and its cache dir.
    pub writable: Vec<PathBuf>,
}

/// Derive the per-mount paths for `account_id`'s confined rclone. `None` when
/// `account_id` is not a safe single path component (empty, `.`/`..`, or with a
/// separator/NUL), so a malformed id can never escape the runtime tree - defence
/// in depth over the config loader's own file-stem pinning.
pub fn mount_paths(
    runtime_dir: &Path,
    account_id: &str,
    mount_point: &Path,
    cache_dir: &Path,
) -> Option<MountPaths> {
    if account_id.is_empty()
        || account_id == "."
        || account_id == ".."
        || account_id.contains(['/', '\0'])
    {
        return None;
    }
    let profile_root = runtime_dir.join("arlen").join("accounts").join(account_id);
    let rc_socket = profile_root.join("rcd.sock");
    let writable =
        vec![profile_root.clone(), mount_point.to_path_buf(), cache_dir.to_path_buf()];
    Some(MountPaths { profile_root, rc_socket, writable })
}

/// The minimal `arlen-run` profile for the confined rclone: its `[info] app_id`,
/// the `[network] allowed_domains` FilteredHosts egress, and the `[filesystem]
/// custom` read-write set. Every other permission section defaults (denied), and
/// `arlen-run`'s loader fills them with `#[serde(default)]` - so this fixed shape
/// is a complete, minimal, least-privilege profile.
#[derive(Serialize)]
struct RcloneProfile {
    info: ProfileInfoOut,
    network: NetworkOut,
    filesystem: FilesystemOut,
}

#[derive(Serialize)]
struct ProfileInfoOut {
    app_id: String,
}

#[derive(Serialize)]
struct NetworkOut {
    allowed_domains: Vec<String>,
}

#[derive(Serialize)]
struct FilesystemOut {
    custom: Vec<String>,
}

/// Render the per-mount profile TOML: FilteredHosts egress to `hosts`, `writable`
/// the read-write filesystem set (the rc socket dir, the mount point, rclone's
/// config dir). The caller supplies a non-empty `hosts` (a host-less backend is
/// refused upstream); nothing else is granted.
pub fn render_rclone_profile(hosts: &[String], writable: &[PathBuf]) -> String {
    let profile = RcloneProfile {
        info: ProfileInfoOut { app_id: RCLONE_APP_ID.to_string() },
        network: NetworkOut { allowed_domains: hosts.to_vec() },
        filesystem: FilesystemOut {
            custom: writable.iter().map(|p| p.to_string_lossy().into_owned()).collect(),
        },
    };
    // A fixed-shape struct of strings cannot fail to serialize to TOML.
    toml::to_string(&profile).expect("serialize the fixed-shape rclone profile")
}

/// Write the per-mount profile to `<profile_root>/{RCLONE_APP_ID}.toml` (the path
/// `arlen-run --profile-root <profile_root> --app-id {RCLONE_APP_ID}` reads),
/// creating `profile_root` `0700` and the file `0600` atomically (temp + rename).
/// Returns the written path.
pub fn write_rclone_profile(
    profile_root: &Path,
    hosts: &[String],
    writable: &[PathBuf],
) -> io::Result<PathBuf> {
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(profile_root)?;
    let toml = render_rclone_profile(hosts, writable);
    let path = profile_root.join(format!("{RCLONE_APP_ID}.toml"));
    let tmp = profile_root.join(format!(".{RCLONE_APP_ID}.toml.tmp"));
    {
        use io::Write;
        let mut f =
            std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(toml.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// A mount resolved and ready to launch: the rclone `fs` + mount point (from the
/// caller-gated [`plan_mount`]), the provider egress allowlist, and the per-mount
/// filesystem layout. The pure decision the D-Bus `Mount` method runs before
/// spawning the confined rclone.
pub struct ResolvedMount {
    /// The rclone connection string + mount point.
    pub plan: MountPlan,
    /// The provider `host:port` set for the FilteredHosts egress profile.
    pub hosts: Vec<String>,
    /// The per-mount runtime paths + writable set.
    pub paths: MountPaths,
}

/// Resolve a mount for `account_id` requested by `caller_app_id`: the caller-auth
/// grant check + the rclone descriptor ([`plan_mount`], refuses an ungranted
/// caller or an account with no `[files]` backend), the provider egress allowlist
/// ([`egress_hosts`], refuses a host-less backend so rclone is never confined to
/// nothing), and the runtime paths ([`mount_paths`]). Fail-closed: any missing
/// piece is a [`MountError`], never a partial mount.
pub fn resolve_mount(
    accounts: &[AccountConfig],
    caller_app_id: &str,
    account_id: &str,
    runtime_dir: &Path,
    cache_dir: &Path,
    secret: Option<&str>,
) -> Result<ResolvedMount, MountError> {
    let plan = plan_mount(accounts, caller_app_id, account_id, runtime_dir, secret)?;
    // plan_mount already verified the grant + a `[files]` backend; take the raw
    // backend for the egress allowlist.
    let backend = accounts
        .iter()
        .find(|a| a.id == account_id)
        .and_then(|a| a.files.as_ref())
        .ok_or(MountError::NoBackend)?;
    let hosts = egress_hosts(backend);
    if hosts.is_empty() {
        // A `[files]` backend that declares no dialable host cannot be scoped to a
        // provider - refuse rather than confine rclone to an empty egress set.
        return Err(MountError::NoBackend);
    }
    let paths =
        mount_paths(runtime_dir, account_id, &plan.mount_point, cache_dir).ok_or(MountError::Refused)?;
    Ok(ResolvedMount { plan, hosts, paths })
}

/// A failure launching or driving the confined rclone mount. The caller maps it
/// to a D-Bus error; every variant leaves nothing half-mounted (the child is
/// killed on drop, `kill_on_drop`).
#[derive(Debug, thiserror::Error)]
pub enum MountLaunchError {
    /// A per-mount runtime/mount/cache dir could not be created.
    #[error("creating a mount dir: {0}")]
    Dir(io::Error),
    /// The confined rclone's permission profile could not be written.
    #[error("writing the rclone profile: {0}")]
    Profile(io::Error),
    /// `arlen-run` (the confiner) could not be spawned.
    #[error("spawning the confined rclone: {0}")]
    Spawn(io::Error),
    /// The confined rclone never opened its rc socket within [`SOCKET_TIMEOUT`].
    #[error("the confined rclone did not open its rc socket in time")]
    SocketTimeout,
    /// The rc API (mount/unmount) failed.
    #[error("driving rclone over the rc api: {0}")]
    Rc(#[from] RcError),
}

/// A live confined rclone mount: the `arlen-run` child (killed on drop), its rc
/// socket, and the FUSE mount point. Held for the mount's lifetime; [`unmount`]
/// tears it down.
pub struct RcloneMount {
    child: Child,
    rc_socket: PathBuf,
    mount_point: PathBuf,
}

impl RcloneMount {
    /// Where the drive is mounted.
    pub fn mount_point(&self) -> &Path {
        &self.mount_point
    }

    /// Unmount the drive and stop the confined rclone. Best-effort teardown: the
    /// child is killed even if the rc `unmount` call fails, so a mount is never
    /// left with a live process.
    pub async fn unmount(mut self) -> Result<(), MountLaunchError> {
        let rc = RcClient::new(UnixRcTransport::new(&self.rc_socket));
        let mp = self.mount_point.to_string_lossy();
        let unmounted = rc.unmount(&mp).await;
        let _ = self.child.kill().await;
        unmounted.map_err(MountLaunchError::Rc)
    }
}

/// Poll for `path` to appear, up to `timeout`. The confined rclone creates its rc
/// socket a moment after it starts, so the daemon waits for it before driving the
/// rc API. Returns whether it appeared.
async fn wait_for_socket(path: &Path, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if path.exists() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    path.exists()
}

/// Launch `rclone` confined under `arlen-run` and mount `plan.fs` at
/// `plan.mount_point`: create the writable dirs (a missing one would be dropped
/// from the Landlock grant), write the per-mount profile with the `hosts`
/// FilteredHosts egress, spawn the confined rclone, wait for its rc socket, then
/// drive `RcClient::mount`. Fail-closed at every step; on any error nothing is
/// left mounted (the child dies on the dropped `Command`/handle).
pub async fn spawn_confined_mount(
    paths: &MountPaths,
    plan: &MountPlan,
    hosts: &[String],
) -> Result<RcloneMount, MountLaunchError> {
    // Landlock only grants a writable path that already exists, so create the
    // whole writable set (the runtime dir, the mount point, the cache dir).
    for dir in &paths.writable {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(MountLaunchError::Dir)?;
    }
    write_rclone_profile(&paths.profile_root, hosts, &paths.writable)
        .map_err(MountLaunchError::Profile)?;

    let argv = confined_rclone_argv(&arlen_run_binary(), &paths.profile_root, &paths.rc_socket);
    let child = Command::new(&argv[0])
        .args(&argv[1..])
        .kill_on_drop(true)
        .spawn()
        .map_err(MountLaunchError::Spawn)?;

    if !wait_for_socket(&paths.rc_socket, SOCKET_TIMEOUT).await {
        return Err(MountLaunchError::SocketTimeout);
    }

    let rc = RcClient::new(UnixRcTransport::new(&paths.rc_socket));
    let mount_point = plan.mount_point.to_string_lossy();
    rc.mount(&plan.fs, &mount_point).await?;

    Ok(RcloneMount {
        child,
        rc_socket: paths.rc_socket.clone(),
        mount_point: plan.mount_point.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sftp(host: Option<&str>, port: Option<u16>) -> FilesBackend {
        FilesBackend {
            backend: ConnectionBackend::Sftp,
            host: host.map(String::from),
            port,
            user: None,
            key_file: None,
            url: None,
            path: None,
        }
    }

    fn webdav(url: Option<&str>) -> FilesBackend {
        FilesBackend {
            backend: ConnectionBackend::Webdav,
            host: None,
            port: None,
            user: None,
            key_file: None,
            url: url.map(String::from),
            path: None,
        }
    }

    #[test]
    fn sftp_egress_defaults_port_22_and_honours_an_explicit_one() {
        assert_eq!(egress_hosts(&sftp(Some("nas.example.org"), None)), vec!["nas.example.org:22"]);
        assert_eq!(egress_hosts(&sftp(Some("nas.example.org"), Some(2222))), vec!["nas.example.org:2222"]);
    }

    #[test]
    fn a_hostless_backend_yields_no_egress_so_the_caller_refuses() {
        assert!(egress_hosts(&sftp(None, None)).is_empty());
        assert!(egress_hosts(&sftp(Some(""), None)).is_empty());
    }

    #[test]
    fn ftp_egress_defaults_port_21() {
        let b = FilesBackend { backend: ConnectionBackend::Ftp, ..sftp(Some("ftp.example.org"), None) };
        assert_eq!(egress_hosts(&b), vec!["ftp.example.org:21"]);
    }

    #[test]
    fn webdav_egress_derives_host_and_port_from_the_url() {
        assert_eq!(egress_hosts(&webdav(Some("https://dav.example.org/remote.php/dav"))), vec!["dav.example.org:443"]);
        assert_eq!(egress_hosts(&webdav(Some("http://dav.example.org:8080/dav"))), vec!["dav.example.org:8080"]);
        // user@ prefix is stripped; the port is kept.
        assert_eq!(egress_hosts(&webdav(Some("https://u@dav.example.org:5006/"))), vec!["dav.example.org:5006"]);
        // A malformed / unsupported URL yields nothing (the caller refuses).
        assert!(egress_hosts(&webdav(Some("ftp://x/y"))).is_empty());
        assert!(egress_hosts(&webdav(Some("not a url"))).is_empty());
        assert!(egress_hosts(&webdav(None)).is_empty());
    }

    const NAS: &str = r#"
        id = "nas"
        provider = "nextcloud"
        identity = "me@nas"
        services = ["files"]

        [[grant]]
        app_id = "org.arlen.files"
        services = ["files"]

        [files]
        backend = "sftp"
        host = "nas.local"
        port = 2222
        user = "me"
    "#;

    fn nas_accounts() -> Vec<AccountConfig> {
        vec![crate::config::parse_account(Path::new("/x/nas.toml"), NAS).unwrap()]
    }

    #[test]
    fn resolve_mount_yields_the_plan_egress_and_paths_for_a_granted_account() {
        let accounts = nas_accounts();
        let rt = PathBuf::from("/run/user/1000");
        let cache = PathBuf::from("/home/u/.cache/arlen/rclone/nas");
        let r = resolve_mount(&accounts, "org.arlen.files", "nas", &rt, &cache, None).unwrap();
        assert_eq!(r.hosts, vec!["nas.local:2222"]);
        assert_eq!(r.plan.mount_point, rt.join("arlen/mounts/nas"));
        assert_eq!(r.paths.profile_root, rt.join("arlen/accounts/nas"));
        assert_eq!(r.paths.rc_socket, r.paths.profile_root.join("rcd.sock"));
    }

    #[test]
    fn resolve_mount_refuses_an_ungranted_caller_before_building_anything() {
        let accounts = nas_accounts();
        let rt = PathBuf::from("/run/user/1000");
        let cache = PathBuf::from("/cache");
        assert!(matches!(
            resolve_mount(&accounts, "other.app", "nas", &rt, &cache, None),
            Err(MountError::Refused)
        ));
    }

    #[tokio::test]
    async fn wait_for_socket_returns_when_it_appears_and_times_out_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("rcd.sock");
        // Absent within the window: the launch is treated as failed (fail-closed).
        assert!(!wait_for_socket(&sock, Duration::from_millis(120)).await);
        // Present: the daemon proceeds to drive the rc API.
        std::fs::write(&sock, b"").unwrap();
        assert!(wait_for_socket(&sock, Duration::from_millis(120)).await);
    }

    #[test]
    fn mount_paths_place_the_socket_and_writable_set_under_the_account_runtime_dir() {
        let rt = PathBuf::from("/run/user/1000");
        let mp = PathBuf::from("/run/user/1000/arlen/mounts/nas");
        let cache = PathBuf::from("/home/u/.cache/arlen/rclone/nas");
        let p = mount_paths(&rt, "nas", &mp, &cache).unwrap();
        assert_eq!(p.profile_root, PathBuf::from("/run/user/1000/arlen/accounts/nas"));
        assert_eq!(p.rc_socket, p.profile_root.join("rcd.sock"));
        // The socket dir, the mount point and the cache dir are all writable so
        // rclone can bind the socket, attach the FUSE mount, and cache.
        assert!(p.writable.contains(&p.profile_root));
        assert!(p.writable.contains(&mp));
        assert!(p.writable.contains(&cache));
    }

    #[test]
    fn a_malformed_account_id_yields_no_mount_paths() {
        let rt = PathBuf::from("/run/user/1000");
        let mp = PathBuf::from("/mp");
        let cache = PathBuf::from("/cache");
        for bad in ["", ".", "..", "a/b", "a\0b"] {
            assert!(mount_paths(&rt, bad, &mp, &cache).is_none(), "must refuse {bad:?}");
        }
    }

    #[test]
    fn the_rendered_profile_loads_through_arlen_runs_own_resolver() {
        let dir = tempfile::tempdir().unwrap();
        let hosts = vec!["nas.example.org:22".to_string()];
        let writable = vec![
            PathBuf::from("/run/user/1000/arlen/accounts/nas"),
            PathBuf::from("/home/u/.cache/rclone"),
        ];
        let path = write_rclone_profile(dir.path(), &hosts, &writable).unwrap();
        assert_eq!(path, dir.path().join(format!("{RCLONE_APP_ID}.toml")));
        // Load the exact file arlen-run reads (it joins `{app_id}.toml` onto the
        // profile root), so the format is validated against the real parser.
        let profile = arlen_permissions::load_profile_from(&path, RCLONE_APP_ID)
            .expect("arlen-run must parse the rendered profile");
        assert_eq!(profile.network.allowed_domains, hosts);
        assert_eq!(
            profile.filesystem.custom, writable,
            "the writable set must round-trip as the [filesystem] custom paths"
        );
        // Least privilege: no blanket network, no home grant.
        assert!(!profile.network.allow_all);
        assert!(!profile.filesystem.home);
    }

    #[test]
    fn the_confined_argv_wraps_rclone_rcd_in_arlen_run_on_the_rc_socket() {
        let argv = confined_rclone_argv(
            &PathBuf::from("/usr/lib/arlen/libexec/arlen-run"),
            &PathBuf::from("/run/x/prof"),
            &PathBuf::from("/run/x/rcd.sock"),
        );
        assert_eq!(argv[0], "/usr/lib/arlen/libexec/arlen-run");
        assert!(argv.windows(2).any(|w| w[0] == "--app-id" && w[1] == RCLONE_APP_ID));
        assert!(argv.windows(2).any(|w| w[0] == "--profile-root" && w[1] == "/run/x/prof"));
        // The confined program is `rclone rcd` on the AF_UNIX rc socket.
        let sep = argv.iter().position(|a| a == "--").unwrap();
        assert_eq!(&argv[sep + 1..sep + 3], &["rclone", "rcd"]);
        assert!(argv.iter().any(|a| a == "--rc-addr=unix:///run/x/rcd.sock"));
        assert!(argv.iter().any(|a| a == "--rc-no-auth"));
    }
}
