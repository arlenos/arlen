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
//! The `Files.Mount` method that provisions the profile, spawns this and calls
//! `RcClient::mount` lands in the next slice; until then these builders are
//! exercised by the unit tests below.
#![allow(dead_code)]

use std::io;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config::FilesBackend;
use crate::connection::ConnectionBackend;

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

/// The `arlen-run` argv that runs `rclone rcd` confined. `arlen-run` reads the
/// per-mount profile at `<profile_root>/{RCLONE_APP_ID}.toml` (Landlock + seccomp
/// + cgroup + the FilteredHosts egress netns), then execs rclone, which serves
/// its rc API on the AF_UNIX `rc_socket`. `--rc-no-auth` is safe: the socket is a
/// private path in the per-mount runtime dir, reachable only by the daemon.
/// Returned as `[program, args...]`; the caller spawns `program` with `args`.
pub fn confined_rclone_argv(profile_root: &Path, rc_socket: &Path) -> Vec<String> {
    vec![
        "arlen-run".to_string(),
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
        let argv = confined_rclone_argv(&PathBuf::from("/run/x/prof"), &PathBuf::from("/run/x/rcd.sock"));
        assert_eq!(argv[0], "arlen-run");
        assert!(argv.windows(2).any(|w| w[0] == "--app-id" && w[1] == RCLONE_APP_ID));
        assert!(argv.windows(2).any(|w| w[0] == "--profile-root" && w[1] == "/run/x/prof"));
        // The confined program is `rclone rcd` on the AF_UNIX rc socket.
        let sep = argv.iter().position(|a| a == "--").unwrap();
        assert_eq!(&argv[sep + 1..sep + 3], &["rclone", "rcd"]);
        assert!(argv.iter().any(|a| a == "--rc-addr=unix:///run/x/rcd.sock"));
        assert!(argv.iter().any(|a| a == "--rc-no-auth"));
    }
}
