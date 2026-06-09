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

use std::collections::HashSet;
use std::net::SocketAddr;

use thiserror::Error;

use crate::{resolve_and_pin, GuardError};

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
}
