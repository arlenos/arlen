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

/// The shell wrapper run inside the namespace, before the app: drop the default
/// route so no public IP is routable (leaving only the private link to the
/// proxy), then exec the app. `CAP_NET_ADMIN` is held inside the userns netns,
/// so the route delete succeeds unprivileged; a failure to delete would leave a
/// route, so it is `exec`-gated - the app runs only after the route is gone.
const ROUTE_ABSENCE_WRAPPER: &str = "ip route del default && exec \"$@\"";

/// Build the full `pasta` argv that runs `app_argv` in a route-absent namespace
/// whose only reachable destination is the host-bound proxy. The returned vector
/// is `[program, args...]`; the caller spawns `program` with `args`.
///
/// `app_argv` is the confined launch (e.g. the `bwrap …` argv), run WITHOUT its
/// own `--unshare-net`: the network namespace is `pasta`'s, the app inherits it.
pub fn pasta_argv(app_argv: &[String]) -> Vec<String> {
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
        // The route-absence wrapper: drop the default route, then exec the app.
        "sh".to_string(),
        "-c".to_string(),
        ROUTE_ABSENCE_WRAPPER.to_string(),
        // $0 (a label; the app argv follows as $1..).
        "arlen-run-netns".to_string(),
    ];
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
        let argv = pasta_argv(&["bwrap".to_string(), "--".to_string(), "app".to_string()]);
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
    fn proxy_binds_on_host_loopback() {
        assert_eq!(proxy_bind_addr(9000).to_string(), "127.0.0.1:9000");
    }

    /// On-kernel proof of the route-absence property: run a probe inside the
    /// pasta namespace and assert it reaches the host-bound proxy but NOT a
    /// public IP. Needs `pasta` + unprivileged userns (the dev machine); run
    /// with `--ignored`. This is the raw-IP-bypass defense the whole enforcer
    /// rests on, verified end to end at the namespace level.
    #[test]
    #[ignore = "needs pasta + unprivileged user namespaces (on-kernel)"]
    fn the_namespace_reaches_only_the_proxy_not_a_public_ip() {
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

        // The probe (the "app"): reach the proxy at the mapped gateway (must
        // succeed) and a public IP (must fail - route absence). bash's /dev/tcp
        // is a pure-shell connect, no extra tools.
        let probe = format!(
            "if : </dev/tcp/{PROXY_NETNS_ADDR}/{port}; then echo PROXY_OK; else echo PROXY_FAIL; fi; \
             if : </dev/tcp/1.1.1.1/80; then echo PUBLIC_OK; else echo PUBLIC_BLOCKED; fi"
        );
        let app_argv = vec!["bash".to_string(), "-c".to_string(), probe];
        let argv = pasta_argv(&app_argv);

        let out = Command::new(&argv[0]).args(&argv[1..]).output().unwrap();
        let _ = accepted.join();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("PROXY_OK"), "proxy unreachable in the netns: {stdout}");
        assert!(
            stdout.contains("PUBLIC_BLOCKED") && !stdout.contains("PUBLIC_OK"),
            "route-absence failed: a public IP was reachable: {stdout}"
        );
    }
}
