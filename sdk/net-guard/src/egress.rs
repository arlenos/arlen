//! Per-host egress filtering: the allowlist vocabulary and (later) the forced
//! forwarding proxy that holds a confined process to its declared
//! `NetworkPolicy::FilteredHosts` set.
//!
//! A hostname allowlist is meaningless at the packet layer (the kernel sees IPs,
//! not names, and a process can `connect()` a raw IP without touching DNS), so the
//! launcher (strand 1) runs the confined process in a network namespace whose only
//! route is a forwarding proxy in the host netns. The proxy owns DNS and `CONNECT`,
//! so it is the single point that maps a requested host to a decision; raw-IP
//! egress is bounded by route absence, not by the proxy. This module is the library
//! half the launcher links: the allowlist matcher here, the verdict + proxy in
//! later commits. The SSRF resolve-and-pin floor ([`crate::resolve_and_pin`]) stays
//! the complementary check - an allowlisted host that resolves into a blocked range
//! is still refused.
//!
//! The launcher's contract is exactly three calls:
//! 1. [`EgressAllowlist::parse`] over the `NetworkPolicy::FilteredHosts` entries.
//! 2. [`EgressProxy::bind`] then [`EgressProxy::listen_addr`] - the launcher writes
//!    that address into the confined `http_proxy`/`https_proxy`/`all_proxy` env and
//!    arranges the netns so it is the ONLY reachable route.
//! 3. `tokio::spawn(proxy.serve(cancel))` - the launcher owns the cancellation
//!    token and cancels on app exit.
//!
//! The route-absence invariant is load-bearing: the proxy is the netns's only
//! route, so a proxy-unaware process that dials a raw IP directly reaches nothing
//! (dropped by route absence, not by the proxy). The proxy enforces the allowlist
//! for proxy-aware traffic; all other egress is dropped because there is no route.
//! That is fail-closed - a hardcoded-IP app fails to reach the network rather than
//! silently bypassing the allowlist. The netns plumbing itself is the launcher's
//! job (strand 1), not this library's.

use std::collections::HashSet;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use std::time::{Duration, Instant};

use thiserror::Error;
use tokio::io::{copy_bidirectional, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::{resolve_and_pin, GuardError, RateLimiter};

/// The maximum bytes the proxy reads while looking for the end of the `CONNECT`
/// request head. A confined process must not be able to make the proxy buffer
/// unboundedly (OOM) by sending a head that never terminates; reaching this cap
/// without a `\r\n\r\n` is a hard reject.
const MAX_REQUEST_HEAD: usize = 8 * 1024;

/// How long the proxy waits for a complete `CONNECT` head before giving up. Bounds
/// a slowloris-style drip (one byte per read, never terminating the head) that
/// would otherwise pin a host task indefinitely on the time axis (the byte cap only
/// bounds memory).
const HEAD_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// The maximum number of concurrent tunnels the proxy serves. A confined process
/// must not exhaust host tasks/FDs by opening connections without bound; beyond
/// this, new connections are dropped (the app's own egress degrades, the host is
/// protected). An owned permit is held for the connection's whole lifetime.
const MAX_CONNECTIONS: usize = 64;

/// A single allowlisted egress destination: an exact `host:port` pair the confined
/// process may reach. Parsed from a `NetworkPolicy::FilteredHosts` entry (the
/// `"api.example.org:443"` wire shape).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AllowedHost {
    /// The DNS host, lowercased (matching is case-insensitive per DNS).
    pub host: String,
    /// The TCP port. Required: an entry without a port is rejected at parse,
    /// because a host allowlist that ignores the port is broader than declared.
    pub port: u16,
}

/// The parsed, deduplicated egress allowlist a confined process is held to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EgressAllowlist {
    hosts: HashSet<AllowedHost>,
}

/// Why an allowlist entry could not be parsed.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AllowlistError {
    /// The entry had no `:port` suffix.
    #[error("allowlist entry `{0}` has no port (expected host:port)")]
    MissingPort(String),
    /// The port was not a valid u16.
    #[error("allowlist entry `{entry}` has a bad port: {port}")]
    BadPort {
        /// The offending entry.
        entry: String,
        /// The unparseable port substring.
        port: String,
    },
    /// The host part was empty.
    #[error("allowlist entry `{0}` has an empty host")]
    EmptyHost(String),
}

impl EgressAllowlist {
    /// Parse a `NetworkPolicy::FilteredHosts` vector into an allowlist. Each entry
    /// must be `host:port`. Fails closed on any malformed entry (a typo must not
    /// silently widen or narrow the set). The host is lowercased so matching is
    /// case-insensitive per DNS; the port must be a valid `u16`.
    pub fn parse(entries: &[String]) -> Result<Self, AllowlistError> {
        let mut hosts = HashSet::new();
        for entry in entries {
            // Split on the LAST colon so an entry is `host:port`; a missing colon is
            // a missing port. (Bare IPv6 literals are not a FilteredHosts shape - the
            // wire form is a DNS host:port, so this is intentionally simple.)
            let (host, port_str) = entry
                .rsplit_once(':')
                .ok_or_else(|| AllowlistError::MissingPort(entry.clone()))?;
            if host.is_empty() {
                return Err(AllowlistError::EmptyHost(entry.clone()));
            }
            let port = port_str.parse::<u16>().map_err(|_| AllowlistError::BadPort {
                entry: entry.clone(),
                port: port_str.to_string(),
            })?;
            hosts.insert(AllowedHost {
                host: host.to_ascii_lowercase(),
                port,
            });
        }
        Ok(EgressAllowlist { hosts })
    }

    /// Whether a `host:port` pair is on the allowlist (case-insensitive host).
    /// Matching is EXACT: a host equals an allowlisted host or it does not, never a
    /// suffix or substring (so `evil-api.example.org` never matches
    /// `api.example.org`).
    pub fn permits(&self, host: &str, port: u16) -> bool {
        self.hosts.contains(&AllowedHost {
            host: host.to_ascii_lowercase(),
            port,
        })
    }

    /// The number of distinct destinations (for logging/telemetry by the launcher).
    pub fn len(&self) -> usize {
        self.hosts.len()
    }

    /// Whether the allowlist is empty (no declared egress destinations).
    pub fn is_empty(&self) -> bool {
        self.hosts.is_empty()
    }
}

/// The verdict the proxy reaches for one requested destination. Not `PartialEq`
/// because [`GuardError`] wraps a `std::io::Error`; match on the variant instead.
#[derive(Debug)]
pub enum EgressVerdict {
    /// On the allowlist and not in a blocked IP range; dial this pinned addr.
    Allow(SocketAddr),
    /// Not on the host allowlist.
    NotAllowlisted,
    /// On the allowlist but resolves into a blocked range (the SSRF floor).
    Blocked(GuardError),
    /// Refused because the per-connection egress rate cap is exhausted (CONN-R3);
    /// the destination may be fine, the process is just talking too fast.
    RateLimited,
}

/// Decide a single requested destination: the host-allowlist check FIRST, then the
/// existing SSRF resolve-and-pin floor. The allowlist check short-circuits before
/// any DNS, so an unlisted host is refused without a lookup (no resolver work, and
/// no side channel from the lookup). An allowlisted host then goes through
/// [`resolve_and_pin`], so an allowlisted name that resolves into a blocked range is
/// still refused. This is the pure decision core the proxy calls per `CONNECT`; it
/// does the DNS (async) but no socket splicing.
pub async fn decide_egress(allowlist: &EgressAllowlist, host: &str, port: u16) -> EgressVerdict {
    if !allowlist.permits(host, port) {
        return EgressVerdict::NotAllowlisted;
    }
    match resolve_and_pin(host, port).await {
        Ok(addr) => EgressVerdict::Allow(addr),
        Err(e) => EgressVerdict::Blocked(e),
    }
}

/// The per-destination decision the proxy runs for each `CONNECT`. The real proxy
/// wires [`decide_egress`] over its allowlist; a test wires a stub so the splice
/// path can be exercised against a loopback upstream without disabling the SSRF
/// floor in production.
type EgressResolver =
    Arc<dyn Fn(String, u16) -> Pin<Box<dyn Future<Output = EgressVerdict> + Send>> + Send + Sync>;

/// Wrap a resolver with a shared egress rate cap: each `CONNECT` first spends a token
/// from `limiter`, and an exhausted bucket short-circuits to [`EgressVerdict::RateLimited`]
/// without consulting `inner` (so a throttled request does no DNS). The lock is taken
/// and released before the inner await, never held across it.
fn with_rate_cap(inner: EgressResolver, limiter: Arc<Mutex<RateLimiter>>) -> EgressResolver {
    Arc::new(move |host, port| {
        let inner = inner.clone();
        let limiter = limiter.clone();
        Box::pin(async move {
            let permitted = limiter
                .lock()
                .map(|mut rl| rl.try_acquire(Instant::now()))
                .unwrap_or(false);
            if !permitted {
                return EgressVerdict::RateLimited;
            }
            inner(host, port).await
        })
    })
}

/// A content-free record of one egress decision for the observer: the requested
/// host + port and the verdict KIND, never the resolved IP or any payload byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EgressDecision {
    /// Allowlisted, in-range and within the rate cap: the tunnel opened.
    Allowed,
    /// Refused: the `host:port` is not on the allowlist.
    NotAllowlisted,
    /// Refused: allowlisted but resolved into a blocked IP range (the SSRF floor).
    Blocked,
    /// Refused: the per-connection rate cap was exhausted.
    RateLimited,
}

impl EgressDecision {
    fn of(verdict: &EgressVerdict) -> Self {
        match verdict {
            EgressVerdict::Allow(_) => EgressDecision::Allowed,
            EgressVerdict::NotAllowlisted => EgressDecision::NotAllowlisted,
            EgressVerdict::Blocked(_) => EgressDecision::Blocked,
            EgressVerdict::RateLimited => EgressDecision::RateLimited,
        }
    }
}

/// Invoked once per `CONNECT` with the requested destination and the decision, so a
/// caller can audit EVERY egress (CONN-R3 "every egress audited"). Content-free and
/// synchronous: net-guard does not depend on the audit ledger, so the consumer (the
/// Connections daemon, arlen-run) wires the ledger into this seam. The default is a
/// no-op; attach one with [`EgressProxy::with_observer`].
pub type EgressObserver = Arc<dyn Fn(&str, u16, EgressDecision) + Send + Sync>;

/// The default observer: does nothing. A confined launch with no audit consumer
/// still works; the Connections daemon attaches a ledger-backed observer.
fn noop_observer() -> EgressObserver {
    Arc::new(|_host, _port, _decision| {})
}

/// A forced forwarding proxy: the confined process's only egress. Speaks HTTP
/// `CONNECT host:port` (tunnelling TLS unbroken) and enforces the allowlist + the
/// SSRF floor on every tunnel. Runs in the host netns; the confined process reaches
/// it over the netns's single link with `*_proxy` env pointed at [`Self::listen_addr`].
pub struct EgressProxy {
    listener: TcpListener,
    resolver: EgressResolver,
    observer: EgressObserver,
}

impl EgressProxy {
    /// Bind the proxy on `bind_addr` (the launcher picks a loopback/veth addr
    /// reachable from the netns) with the given allowlist. Does not yet serve. Each
    /// `CONNECT` is decided by [`decide_egress`] over `allowlist`.
    pub async fn bind(bind_addr: SocketAddr, allowlist: EgressAllowlist) -> std::io::Result<Self> {
        let resolver: EgressResolver = Arc::new(move |host, port| {
            let allowlist = allowlist.clone();
            Box::pin(async move { decide_egress(&allowlist, &host, port).await })
        });
        Self::bind_with_resolver(bind_addr, resolver).await
    }

    /// Bind with a per-connection egress rate cap of `requests_per_minute` (CONN-R3,
    /// ~60/min Envoy-style) on top of the allowlist + SSRF floor. The cap is the whole
    /// proxy's budget (one confined process), token-bucket with a `requests_per_minute`
    /// burst; a `CONNECT` over the cap is refused [`EgressVerdict::RateLimited`] BEFORE
    /// any DNS, so a flooding process throttles itself rather than the host. The
    /// uncapped [`Self::bind`] stays for callers that do not set a policy.
    pub async fn bind_capped(
        bind_addr: SocketAddr,
        allowlist: EgressAllowlist,
        requests_per_minute: u32,
    ) -> std::io::Result<Self> {
        let base: EgressResolver = Arc::new(move |host, port| {
            let allowlist = allowlist.clone();
            Box::pin(async move { decide_egress(&allowlist, &host, port).await })
        });
        let limiter = Arc::new(Mutex::new(RateLimiter::per_minute(
            requests_per_minute,
            Instant::now(),
        )));
        Self::bind_with_resolver(bind_addr, with_rate_cap(base, limiter)).await
    }

    /// Bind with an injected decision function. Private (not a public constructor)
    /// so the ONLY production entry point is [`Self::bind`], which always wraps
    /// [`decide_egress`] and therefore cannot skip the SSRF floor; the in-crate
    /// tests use this seam to return `Allow(loopback)` and exercise the splice path
    /// without relaxing the floor in production.
    async fn bind_with_resolver(
        bind_addr: SocketAddr,
        resolver: EgressResolver,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind(bind_addr).await?;
        Ok(Self { listener, resolver, observer: noop_observer() })
    }

    /// Attach an egress observer: it is invoked once per `CONNECT` with the
    /// requested `(host, port)` and the [`EgressDecision`], so the caller can audit
    /// every egress without net-guard depending on the audit ledger.
    pub fn with_observer(mut self, observer: EgressObserver) -> Self {
        self.observer = observer;
        self
    }

    /// The address the launcher writes into the confined `http(s)_proxy` env.
    pub fn listen_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Serve until cancelled. Each accepted connection is one `CONNECT` tunnel:
    /// parse the request head, decide, on `Allow` splice both directions to the
    /// pinned addr, otherwise reply `403`/`502` and close. Cancellation-safe via
    /// `cancel` (the launcher cancels on app exit); a per-connection error never
    /// stops the accept loop.
    pub async fn serve(self, cancel: CancellationToken) {
        let limit = Arc::new(Semaphore::new(MAX_CONNECTIONS));
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                accepted = self.listener.accept() => {
                    let Ok((stream, _peer)) = accepted else { continue };
                    // Cap concurrent tunnels: an owned permit is held for the whole
                    // connection. At the cap, drop the connection (the app's egress
                    // degrades; the host task/FD table is protected).
                    let Ok(permit) = limit.clone().try_acquire_owned() else { continue };
                    let resolver = self.resolver.clone();
                    let observer = self.observer.clone();
                    tokio::spawn(async move {
                        let _ = handle_tunnel(stream, resolver, observer).await;
                        drop(permit);
                    });
                }
            }
        }
    }
}

/// Handle one `CONNECT` tunnel: read the bounded request head, parse the target,
/// decide, and either splice to the pinned upstream or reply with the refusal.
async fn handle_tunnel(
    mut client: TcpStream,
    resolver: EgressResolver,
    observer: EgressObserver,
) -> std::io::Result<()> {
    // Bound the head read on the time axis (the byte cap bounds only memory): a
    // peer that never completes the head is reaped.
    let parsed = match tokio::time::timeout(HEAD_READ_TIMEOUT, read_connect_target(&mut client)).await
    {
        Ok(Ok(target)) => target,
        _ => {
            // Malformed, oversized or timed-out head: refuse, do not dial anything.
            let _ = client.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n").await;
            return Ok(());
        }
    };
    let (host, port, early_data) = parsed;

    // Decide, then notify the observer (audit every egress) before acting on it.
    let verdict = resolver(host.clone(), port).await;
    observer(&host, port, EgressDecision::of(&verdict));
    match verdict {
        EgressVerdict::Allow(addr) => {
            let mut upstream = match TcpStream::connect(addr).await {
                Ok(u) => u,
                Err(_) => {
                    let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
                    return Ok(());
                }
            };
            client
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await?;
            // Any bytes the client coalesced after the CONNECT head are tunnel body
            // already consumed off the socket; forward them before splicing or the
            // first request (e.g. a TLS ClientHello) would be truncated.
            if !early_data.is_empty() {
                upstream.write_all(&early_data).await?;
            }
            // Splice both directions until both halves close; the sockets drop when
            // this returns, so no upstream leak on a half-close.
            let _ = copy_bidirectional(&mut client, &mut upstream).await;
        }
        EgressVerdict::NotAllowlisted => {
            let _ = client.write_all(b"HTTP/1.1 403 Forbidden\r\n\r\n").await;
        }
        EgressVerdict::Blocked(_) => {
            let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
        }
        EgressVerdict::RateLimited => {
            let _ = client
                .write_all(b"HTTP/1.1 429 Too Many Requests\r\n\r\n")
                .await;
        }
    }
    Ok(())
}

/// Read the `CONNECT` request head (bounded by [`MAX_REQUEST_HEAD`]) and return the
/// requested `(host, port, early_data)`, where `early_data` is any bytes the client
/// coalesced after the `\r\n\r\n` head terminator (tunnel body the caller must
/// forward before splicing). Errors on a non-`CONNECT` method, a malformed
/// authority, a missing/invalid port, or a head that exceeds the cap without
/// terminating - in every error case the caller refuses without dialling.
async fn read_connect_target(client: &mut TcpStream) -> Result<(String, u16, Vec<u8>), ()> {
    let mut buf: Vec<u8> = Vec::with_capacity(256);
    let mut chunk = [0u8; 1024];
    loop {
        // Find the end of the head.
        if let Some(pos) = find_head_end(&buf) {
            let (host, port) = parse_connect_line(&buf[..pos])?;
            let early_data = buf[pos + 4..].to_vec();
            return Ok((host, port, early_data));
        }
        if buf.len() >= MAX_REQUEST_HEAD {
            return Err(());
        }
        let n = client.read(&mut chunk).await.map_err(|_| ())?;
        if n == 0 {
            return Err(()); // EOF before a complete head
        }
        buf.extend_from_slice(&chunk[..n.min(MAX_REQUEST_HEAD - buf.len())]);
    }
}

/// The index of the first byte of the `\r\n\r\n` head terminator, if present.
fn find_head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

/// Parse the first request line out of the head bytes: `CONNECT host:port HTTP/1.1`.
/// Mis-parses fail closed (any `Err` becomes a 400 with no dial). An IPv6-literal
/// authority (`[::1]:443`) keeps its brackets and so never matches a DNS-host
/// allowlist entry, and a bare-colon v6 form splits the same way the allowlist
/// parser does, so the parser and the vocabulary agree - no entry can be smuggled
/// past the allowlist by authority shape.
fn parse_connect_line(head: &[u8]) -> Result<(String, u16), ()> {
    let text = std::str::from_utf8(head).map_err(|_| ())?;
    let first = text.lines().next().ok_or(())?;
    let mut parts = first.split_whitespace();
    let method = parts.next().ok_or(())?;
    let authority = parts.next().ok_or(())?;
    if !method.eq_ignore_ascii_case("CONNECT") {
        return Err(());
    }
    let (host, port_str) = authority.rsplit_once(':').ok_or(())?;
    if host.is_empty() {
        return Err(());
    }
    let port = port_str.parse::<u16>().map_err(|_| ())?;
    Ok((host.to_string(), port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_host_port_entry() {
        let a = EgressAllowlist::parse(&["api.example.org:443".to_string()]).unwrap();
        assert_eq!(a.len(), 1);
        assert!(a.permits("api.example.org", 443));
    }

    #[test]
    fn rejects_a_missing_port() {
        assert_eq!(
            EgressAllowlist::parse(&["api.example.org".to_string()]),
            Err(AllowlistError::MissingPort("api.example.org".into()))
        );
    }

    #[test]
    fn rejects_a_bad_port() {
        assert_eq!(
            EgressAllowlist::parse(&["api.example.org:notaport".to_string()]),
            Err(AllowlistError::BadPort {
                entry: "api.example.org:notaport".into(),
                port: "notaport".into(),
            })
        );
    }

    #[test]
    fn rejects_an_empty_host() {
        assert_eq!(
            EgressAllowlist::parse(&[":443".to_string()]),
            Err(AllowlistError::EmptyHost(":443".into()))
        );
    }

    #[test]
    fn host_match_is_case_insensitive() {
        let a = EgressAllowlist::parse(&["api.example.org:443".to_string()]).unwrap();
        assert!(a.permits("API.Example.ORG", 443));
    }

    #[test]
    fn port_discriminates() {
        let a = EgressAllowlist::parse(&["api.example.org:443".to_string()]).unwrap();
        assert!(!a.permits("api.example.org", 80));
    }

    #[test]
    fn match_is_exact_not_suffix() {
        let a = EgressAllowlist::parse(&["api.example.org:443".to_string()]).unwrap();
        assert!(!a.permits("evil-api.example.org", 443));
        assert!(!a.permits("api.example.org.evil.com", 443));
    }

    #[test]
    fn duplicate_entries_dedup() {
        let a = EgressAllowlist::parse(&[
            "api.example.org:443".to_string(),
            "API.EXAMPLE.ORG:443".to_string(),
        ])
        .unwrap();
        assert_eq!(a.len(), 1, "case-folded duplicates collapse");
    }

    #[test]
    fn empty_allowlist_is_empty() {
        let a = EgressAllowlist::parse(&[]).unwrap();
        assert!(a.is_empty());
    }

    #[tokio::test]
    async fn decide_short_circuits_before_dns_for_an_unlisted_host() {
        // A host not on the allowlist is refused as NotAllowlisted, not Blocked -
        // proving the allowlist check ran before any resolve. The probe host would
        // resolve to loopback (a Blocked verdict) if DNS had run.
        let a = EgressAllowlist::parse(&["other.example:443".to_string()]).unwrap();
        let v = decide_egress(&a, "127.0.0.1", 443).await;
        assert!(
            matches!(v, EgressVerdict::NotAllowlisted),
            "got {v:?}, expected NotAllowlisted (allowlist check precedes DNS)"
        );
    }

    #[tokio::test]
    async fn decide_allows_an_allowlisted_public_literal() {
        let a = EgressAllowlist::parse(&["8.8.8.8:443".to_string()]).unwrap();
        let v = decide_egress(&a, "8.8.8.8", 443).await;
        match v {
            EgressVerdict::Allow(addr) => {
                assert_eq!(addr, "8.8.8.8:443".parse::<SocketAddr>().unwrap())
            }
            other => panic!("expected Allow, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn decide_blocks_an_allowlisted_loopback() {
        // The SSRF floor still fires: an allowlisted host that resolves into a
        // blocked range is refused.
        let a = EgressAllowlist::parse(&["127.0.0.1:443".to_string()]).unwrap();
        let v = decide_egress(&a, "127.0.0.1", 443).await;
        assert!(
            matches!(v, EgressVerdict::Blocked(_)),
            "got {v:?}, expected Blocked (SSRF floor)"
        );
    }

    // ── proxy tests (commit 3) ──

    /// A resolver that always allows, dialling the given fixed upstream. Lets the
    /// splice path run against a loopback upstream without relaxing the SSRF floor.
    fn allow_resolver(upstream: SocketAddr) -> EgressResolver {
        Arc::new(move |_host, _port| Box::pin(async move { EgressVerdict::Allow(upstream) }))
    }

    /// A resolver that always refuses as NotAllowlisted.
    fn deny_resolver() -> EgressResolver {
        Arc::new(|_host, _port| Box::pin(async { EgressVerdict::NotAllowlisted }))
    }

    async fn read_status_line(stream: &mut TcpStream) -> String {
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        // Read until the first \n (end of the status line).
        loop {
            let n = stream.read(&mut byte).await.unwrap();
            if n == 0 {
                break;
            }
            buf.push(byte[0]);
            if byte[0] == b'\n' {
                break;
            }
        }
        String::from_utf8_lossy(&buf).trim().to_string()
    }

    #[tokio::test]
    async fn the_observer_records_each_egress_decision() {
        // A refused CONNECT still notifies the observer with the requested
        // destination + the decision kind, so the caller can audit every egress.
        let seen = Arc::new(Mutex::new(Vec::<(String, u16, EgressDecision)>::new()));
        let sink = seen.clone();
        let observer: EgressObserver = Arc::new(move |host, port, decision| {
            sink.lock().unwrap().push((host.to_string(), port, decision));
        });

        let proxy = EgressProxy::bind_with_resolver("127.0.0.1:0".parse().unwrap(), deny_resolver())
            .await
            .unwrap()
            .with_observer(observer);
        let proxy_addr = proxy.listen_addr().unwrap();
        let cancel = CancellationToken::new();
        let c2 = cancel.clone();
        tokio::spawn(async move { proxy.serve(c2).await });

        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        client
            .write_all(b"CONNECT api.example.org:443 HTTP/1.1\r\n\r\n")
            .await
            .unwrap();
        let status = read_status_line(&mut client).await;
        assert!(status.contains("403"), "expected 403, got {status:?}");

        // The observer fires before the refusal is written, so it has recorded
        // exactly the requested destination + the NotAllowlisted kind by now.
        let records = seen.lock().unwrap().clone();
        assert_eq!(
            records,
            vec![("api.example.org".to_string(), 443, EgressDecision::NotAllowlisted)]
        );
    }

    #[tokio::test]
    async fn allowlisted_connect_tunnels_bytes() {
        // A fake upstream that echoes one line back.
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut s, _) = upstream.accept().await.unwrap();
            let mut b = [0u8; 5];
            s.read_exact(&mut b).await.unwrap();
            s.write_all(&b).await.unwrap();
        });

        let proxy = EgressProxy::bind_with_resolver(
            "127.0.0.1:0".parse().unwrap(),
            allow_resolver(upstream_addr),
        )
        .await
        .unwrap();
        let proxy_addr = proxy.listen_addr().unwrap();
        let cancel = CancellationToken::new();
        let c2 = cancel.clone();
        tokio::spawn(async move { proxy.serve(c2).await });

        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        client
            .write_all(b"CONNECT api.example.org:443 HTTP/1.1\r\n\r\n")
            .await
            .unwrap();
        let status = read_status_line(&mut client).await;
        assert!(status.contains("200"), "expected 200, got {status:?}");
        // Consume the rest of the blank-line terminator, then tunnel bytes.
        let mut rest = [0u8; 2];
        client.read_exact(&mut rest).await.unwrap(); // the trailing \r\n
        client.write_all(b"hello").await.unwrap();
        let mut echoed = [0u8; 5];
        client.read_exact(&mut echoed).await.unwrap();
        assert_eq!(&echoed, b"hello", "bytes tunnel through to the upstream");
        cancel.cancel();
    }

    #[tokio::test]
    async fn coalesced_early_data_is_forwarded() {
        // The client coalesces the CONNECT head and the first tunnel bytes in one
        // write; the proxy must forward those early bytes, not drop them.
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut s, _) = upstream.accept().await.unwrap();
            let mut b = [0u8; 5];
            s.read_exact(&mut b).await.unwrap();
            s.write_all(&b).await.unwrap();
        });

        let proxy = EgressProxy::bind_with_resolver(
            "127.0.0.1:0".parse().unwrap(),
            allow_resolver(upstream_addr),
        )
        .await
        .unwrap();
        let proxy_addr = proxy.listen_addr().unwrap();
        let cancel = CancellationToken::new();
        let c2 = cancel.clone();
        tokio::spawn(async move { proxy.serve(c2).await });

        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        // Head AND early tunnel bytes in a single write.
        client
            .write_all(b"CONNECT api.example.org:443 HTTP/1.1\r\n\r\nhello")
            .await
            .unwrap();
        let status = read_status_line(&mut client).await;
        assert!(status.contains("200"), "expected 200, got {status:?}");
        let mut rest = [0u8; 2];
        client.read_exact(&mut rest).await.unwrap(); // trailing \r\n
        let mut echoed = [0u8; 5];
        client.read_exact(&mut echoed).await.unwrap();
        assert_eq!(&echoed, b"hello", "coalesced early data reaches the upstream");
        cancel.cancel();
    }

    #[tokio::test]
    async fn the_rate_cap_refuses_after_the_burst() {
        // A 2-token bucket that never refills, wrapping a resolver that would otherwise
        // pass: the first two CONNECTs reach the inner verdict, the third is refused by
        // the cap (CONN-R3) without consulting the inner resolver.
        let limiter = Arc::new(Mutex::new(RateLimiter::new(2, 0.0, Instant::now())));
        let capped = with_rate_cap(deny_resolver(), limiter);
        assert!(matches!(capped("h".into(), 80).await, EgressVerdict::NotAllowlisted));
        assert!(matches!(capped("h".into(), 80).await, EgressVerdict::NotAllowlisted));
        assert!(matches!(capped("h".into(), 80).await, EgressVerdict::RateLimited));
    }

    #[tokio::test]
    async fn non_allowlisted_connect_is_refused_and_never_dials() {
        // The deny resolver never yields an upstream; assert a 403 and that no dial
        // happens (there is no upstream to dial, and the resolver short-circuits).
        let proxy = EgressProxy::bind_with_resolver(
            "127.0.0.1:0".parse().unwrap(),
            deny_resolver(),
        )
        .await
        .unwrap();
        let proxy_addr = proxy.listen_addr().unwrap();
        let cancel = CancellationToken::new();
        let c2 = cancel.clone();
        tokio::spawn(async move { proxy.serve(c2).await });

        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        client
            .write_all(b"CONNECT blocked.example:443 HTTP/1.1\r\n\r\n")
            .await
            .unwrap();
        let status = read_status_line(&mut client).await;
        assert!(status.contains("403"), "expected 403, got {status:?}");
        cancel.cancel();
    }

    #[tokio::test]
    async fn an_unterminated_head_is_rejected() {
        // A head that never terminates must not buffer unboundedly; the parser caps
        // and the connection is refused with 400.
        let proxy = EgressProxy::bind_with_resolver(
            "127.0.0.1:0".parse().unwrap(),
            allow_resolver("127.0.0.1:1".parse().unwrap()),
        )
        .await
        .unwrap();
        let proxy_addr = proxy.listen_addr().unwrap();
        let cancel = CancellationToken::new();
        let c2 = cancel.clone();
        tokio::spawn(async move { proxy.serve(c2).await });

        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        // Send a CONNECT line but no terminating blank line, then a flood of bytes.
        client.write_all(b"CONNECT api.example.org:443 HTTP/1.1\r\n").await.unwrap();
        let flood = vec![b'x'; 16 * 1024];
        let _ = client.write_all(&flood).await;
        let status = read_status_line(&mut client).await;
        assert!(status.contains("400"), "expected 400, got {status:?}");
        cancel.cancel();
    }
}
