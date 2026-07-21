//! The per-launch network namespace that gives a `FilteredHosts` app its
//! route-absence: the raw-IP bypass defense that makes the forwarding
//! `EgressProxy` the app's ONLY reachable destination.
//!
//! The mechanism, validated on-kernel (rootless, no `CAP_NET_ADMIN` in the
//! init namespace):
//!
//! 1. `pasta` (from `passt`) runs the app in a fresh network namespace and
//!    connects it to the host over an unprivileged userspace tap. It is given a
//!    PRIVATE point-to-point subnet ([`NETNS_ADDR`]/[`NETNS_PREFIX`], gateway
//!    [`PROXY_NETNS_ADDR`]) so the namespace has NO presence on the real LAN -
//!    only the private link exists.
//! 2. `--map-host-loopback` [`PROXY_NETNS_ADDR`] makes that gateway address
//!    refer to the HOST's loopback (port-preserving), so the app reaches the
//!    `EgressProxy` (bound on the host at `127.0.0.1:PORT`) at
//!    `PROXY_NETNS_ADDR:PORT`.
//! 3. The launch wrapper removes the default route inside the namespace before
//!    exec'ing the app (`CAP_NET_ADMIN` is held inside the userns netns), so a
//!    direct `connect()` to any public IP is `ENETUNREACH`. Only the private
//!    subnet - which contains nothing but the mapped-loopback gateway (the
//!    proxy) - remains routable. Route-absence for everything else.
//!
//! The net effect: HTTP(S) succeeds only through the proxy (via the
//! `*_proxy` env the launcher sets), and a raw-IP dial reaches nothing. This
//! module builds the exact `pasta` invocation; the launcher binds the proxy,
//! spawns this, and holds the teardown guard.
//!
//! The enforcer that binds the proxy and spawns this invocation lands in the
//! next slice (it needs an async runtime for `EgressProxy`); until then the
//! builders are exercised by the unit + on-kernel tests below.
#![allow(dead_code)]

use std::net::SocketAddr;

/// The private address assigned to the app's network namespace. A
/// point-to-point `/30`-class private link (see [`NETNS_PREFIX`]) with no real
/// LAN presence; only the gateway ([`PROXY_NETNS_ADDR`]) is a live peer.
pub const NETNS_ADDR: &str = "10.99.99.2";

/// The gateway address inside the namespace. `--map-host-loopback` binds it to
/// the host's loopback, so this is where the app reaches the host-bound
/// `EgressProxy` (the proxy's port is preserved through the translation).
pub const PROXY_NETNS_ADDR: &str = "10.99.99.1";

/// The prefix length of the private link. A `/24` whose only live address is
/// the gateway/proxy; after the default route is dropped, this subnet route is
/// all that remains, so the proxy is reachable and nothing else is.
pub const NETNS_PREFIX: &str = "24";

/// The `http_proxy`/`https_proxy`/`all_proxy` URL the launcher sets in the
/// confined environment: the proxy, reached at the mapped-loopback gateway on
/// the port the host-side `EgressProxy` bound.
pub fn proxy_env_url(proxy_port: u16) -> String {
    format!("http://{PROXY_NETNS_ADDR}:{proxy_port}")
}

/// The file descriptor the seccomp wrapper opens the app filter on, inside the
/// namespace. `bwrap --seccomp` reads its cBPF from this fd; the caller puts
/// `--seccomp <SECCOMP_WRAPPER_FD>` in the app argv. It is opened INSIDE the
/// wrapper (not inherited from the launcher) because `pasta` does not pass the
/// launcher's fds through to its child - an inherited memfd would be gone.
pub const SECCOMP_WRAPPER_FD: i32 = 3;

/// The shell wrapper run inside the namespace, before the app: drop the default
/// route so no public IP is routable (leaving only the private link to the
/// proxy), then exec the app. `CAP_NET_ADMIN` is held inside the userns netns,
/// so the route delete succeeds unprivileged; a failure to delete would leave a
/// route, so it is `exec`-gated - the app runs only after the route is gone.
const ROUTE_ABSENCE_WRAPPER: &str = "ip route del default && exec \"$@\"";

/// The route-absence wrapper plus seccomp delivery: open the app filter file
/// (passed as `$1`) on fd 3, drop it from the args, then drop the route and exec
/// the app (whose argv carries `--seccomp 3`). Every step is `&&`-gated, so a
/// missing/unreadable filter or a failed route delete refuses the launch (the
/// app never execs) - fail-closed, never a layer short.
const SECCOMP_ROUTE_WRAPPER: &str =
    "exec 3<\"$1\" && shift && ip route del default && exec \"$@\"";

/// Build the full `pasta` argv that runs `app_argv` in a route-absent namespace
/// whose only reachable destination is the host-bound proxy. The returned vector
/// is `[program, args...]`; the caller spawns `program` with `args`.
///
/// `app_argv` is the confined launch (e.g. the `bwrap …` argv), run WITHOUT its
/// own `--unshare-net`: the network namespace is `pasta`'s, the app inherits it.
/// `seccomp_file`, when given, is the app's compiled cBPF filter: the wrapper
/// opens it on [`SECCOMP_WRAPPER_FD`] inside the namespace (since `pasta` drops
/// the launcher's fds), so `bwrap --seccomp 3` in `app_argv` installs it.
pub fn pasta_argv(app_argv: &[String], seccomp_file: Option<&str>) -> Vec<String> {
    let mut argv = vec![
        "pasta".to_string(),
        // Configure the namespace's interface, address and routes from the
        // host template, then override the address/gateway to the private link.
        "--config-net".to_string(),
        "--address".to_string(),
        NETNS_ADDR.to_string(),
        "--netmask".to_string(),
        NETNS_PREFIX.to_string(),
        "--gateway".to_string(),
        PROXY_NETNS_ADDR.to_string(),
        // The gateway address refers to the host's loopback (where the proxy
        // binds), port-preserving.
        "--map-host-loopback".to_string(),
        PROXY_NETNS_ADDR.to_string(),
        "--".to_string(),
        "sh".to_string(),
        "-c".to_string(),
    ];
    match seccomp_file {
        Some(file) => {
            argv.push(SECCOMP_ROUTE_WRAPPER.to_string());
            // $0 label, then $1 = the seccomp file the wrapper opens + shifts off.
            argv.push("arlen-run-netns".to_string());
            argv.push(file.to_string());
        }
        None => {
            argv.push(ROUTE_ABSENCE_WRAPPER.to_string());
            argv.push("arlen-run-netns".to_string());
        }
    }
    argv.extend(app_argv.iter().cloned());
    argv
}

/// The host address the proxy should bind on so the mapped-loopback translation
/// reaches it: loopback, on the given port (the app dials it at
/// [`PROXY_NETNS_ADDR`] via `--map-host-loopback`).
pub fn proxy_bind_addr(proxy_port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], proxy_port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_env_url_points_at_the_mapped_gateway_and_port() {
        assert_eq!(proxy_env_url(8080), "http://10.99.99.1:8080");
    }

    #[test]
    fn pasta_argv_carries_the_private_link_and_wraps_the_app() {
        let argv = pasta_argv(&["bwrap".to_string(), "--".to_string(), "app".to_string()], None);
        assert_eq!(argv[0], "pasta");
        // The private link + mapped loopback are present.
        assert!(argv.windows(2).any(|w| w[0] == "--address" && w[1] == NETNS_ADDR));
        assert!(argv.windows(2).any(|w| w[0] == "--gateway" && w[1] == PROXY_NETNS_ADDR));
        assert!(argv
            .windows(2)
            .any(|w| w[0] == "--map-host-loopback" && w[1] == PROXY_NETNS_ADDR));
        // The route-absence wrapper drops the default route before exec.
        assert!(argv.iter().any(|a| a == ROUTE_ABSENCE_WRAPPER));
        assert!(ROUTE_ABSENCE_WRAPPER.contains("ip route del default"));
        // The app argv is appended after the wrapper's $0 label.
        let tail: Vec<&String> = argv.iter().rev().take(3).collect();
        assert_eq!(tail, vec![&"app".to_string(), &"--".to_string(), &"bwrap".to_string()]);
    }

    #[test]
    fn pasta_argv_with_a_seccomp_file_opens_it_before_the_app() {
        let argv = pasta_argv(&["bwrap".to_string(), "app".to_string()], Some("/run/seccomp.bpf"));
        // The seccomp wrapper opens fd 3 from $1 and drops it before the app.
        assert!(argv.iter().any(|a| a == SECCOMP_ROUTE_WRAPPER));
        assert!(SECCOMP_ROUTE_WRAPPER.contains(&format!("exec {SECCOMP_WRAPPER_FD}<")));
        assert!(SECCOMP_ROUTE_WRAPPER.contains("ip route del default"));
        // The filter path is the wrapper's $1 (right after the $0 label), then
        // the app argv follows.
        let label = argv.iter().position(|a| a == "arlen-run-netns").unwrap();
        assert_eq!(argv[label + 1], "/run/seccomp.bpf");
        assert_eq!(argv[label + 2], "bwrap");
        assert_eq!(argv.last().unwrap(), "app");
    }

    #[test]
    fn proxy_binds_on_host_loopback() {
        assert_eq!(proxy_bind_addr(9000).to_string(), "127.0.0.1:9000");
    }

    /// On-kernel proof of the route-absence property in the PRODUCTION layering:
    /// the probe runs inside a real `bwrap` confinement (as the app will), itself
    /// inside the pasta namespace, and must reach the host-bound proxy but NOT a
    /// public IP. This proves the whole composition - `pasta` (private link +
    /// route drop in the wrapper's own userns, which owns the namespace) wrapping
    /// `bwrap` (its own userns/mount setup) - achieves the raw-IP-bypass defense.
    /// Needs `pasta` + `bwrap` + unprivileged userns (the dev machine); run with
    /// `--ignored`.
    #[test]
    #[ignore = "needs pasta + bwrap + unprivileged user namespaces (on-kernel)"]
    fn a_bwrapped_app_in_the_namespace_reaches_only_the_proxy() {
        use std::io::Read;
        use std::net::TcpListener;
        use std::process::Command;

        // A host-loopback stand-in for the EgressProxy: accept one connection.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let accepted = std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                let mut buf = [0u8; 1];
                let _ = s.read(&mut buf);
            }
        });

        // The probe: reach the proxy at the mapped gateway (must succeed) and a
        // public IP (must fail - route absence). bash's /dev/tcp is a pure-shell
        // connect, no extra tools. NB the route drop is NOT here: it belongs to
        // the pasta wrapper (which owns the namespace's userns); inside bwrap's
        // fresh userns it would silently fail (no CAP_NET_ADMIN over that netns).
        let probe = format!(
            "if : </dev/tcp/{PROXY_NETNS_ADDR}/{port}; then echo PROXY_OK; else echo PROXY_FAIL; fi; \
             if : </dev/tcp/1.1.1.1/80; then echo PUBLIC_OK; else echo PUBLIC_BLOCKED; fi"
        );
        // The confined "app": bwrap (as the launcher spawns it) running the probe.
        let app_argv: Vec<String> = [
            "bwrap",
            "--unshare-user",
            "--unshare-pid",
            "--ro-bind",
            "/",
            "/",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--",
            "bash",
            "-c",
            &probe,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let argv = pasta_argv(&app_argv, None);

        let out = Command::new(&argv[0]).args(&argv[1..]).output().unwrap();
        let _ = accepted.join();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("PROXY_OK"), "proxy unreachable from the bwrapped app: {stdout}");
        assert!(
            stdout.contains("PUBLIC_BLOCKED") && !stdout.contains("PUBLIC_OK"),
            "route-absence failed: a public IP was reachable from the bwrapped app: {stdout}"
        );
    }

    /// On-kernel proof that the app's real seccomp filter reaches `bwrap` through
    /// `pasta` via the FILE the wrapper opens (the memfd path breaks because
    /// `pasta` drops inherited fds). The wrapper opens the compiled cBPF on fd 3,
    /// `bwrap --seccomp 3` installs it, and the app runs - so the app runs with
    /// the seccomp layer, not a layer short. If the fd delivery failed, `bwrap`
    /// would error on the bad `--seccomp` fd and the marker would be absent.
    #[test]
    #[ignore = "needs pasta + bwrap + unprivileged user namespaces (on-kernel)"]
    fn the_seccomp_filter_reaches_bwrap_through_the_wrapper_file() {
        use std::io::Write;
        use std::process::Command;

        // The real app filter, written where the wrapper can open it.
        let bpf = crate::seccomp::app_filter_bytes().expect("compile the app seccomp filter");
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(&bpf).unwrap();
        let path = f.path().to_str().unwrap().to_string();

        // bwrap installs the filter from fd 3 (the wrapper opens it), then runs a
        // trivial allowlisted app (echo) - if the filter installed, the marker
        // prints; a bad fd makes bwrap fail before the app runs.
        let app_argv: Vec<String> = [
            "bwrap",
            "--seccomp",
            &SECCOMP_WRAPPER_FD.to_string(),
            "--unshare-user",
            "--unshare-pid",
            "--ro-bind",
            "/",
            "/",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--",
            "echo",
            "SECCOMP_INSTALLED",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let argv = pasta_argv(&app_argv, Some(&path));

        let out = Command::new(&argv[0]).args(&argv[1..]).output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("SECCOMP_INSTALLED"),
            "the app did not run - bwrap likely rejected the seccomp fd delivered through pasta: \
             stdout={stdout} stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
