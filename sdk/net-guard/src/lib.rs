//! SSRF guard for outbound fetches.
//!
//! A hostname allowlist alone is not enough: a DNS-controlled name on the
//! allowlist could resolve to a loopback or RFC1918 address and reach an
//! internal service. This crate provides the destination-IP blocklist and a
//! resolve-and-pin step that rejects any host resolving into a blocked range
//! and returns a single verified socket address the caller pins its HTTP client
//! to, closing the DNS-rebinding window between the check and the connect.
//!
//! Lifted from the modulesd network host import so the forage fetch phase and
//! any other outbound caller share one hardened implementation.

use std::net::{IpAddr, SocketAddr};

use thiserror::Error;

mod egress;
pub use egress::{
    decide_egress, AllowedHost, AllowlistError, EgressAllowlist, EgressProxy, EgressVerdict,
};

mod rate;
pub use rate::RateLimiter;

/// Why a destination was refused before any socket was opened.
#[derive(Debug, Error)]
pub enum GuardError {
    /// DNS resolution of the host failed.
    #[error("resolve {host}: {source}")]
    Resolve {
        /// The host that failed to resolve.
        host: String,
        /// The underlying resolver error.
        source: std::io::Error,
    },
    /// The host resolved to no addresses.
    #[error("no addresses for {0}")]
    NoAddresses(String),
    /// A resolved address falls into a blocked range.
    #[error("destination {ip} (host {host}) is in a blocked range")]
    Blocked {
        /// The blocked address.
        ip: IpAddr,
        /// The host that resolved to it.
        host: String,
    },
}

/// Whether an address is one a sandboxed outbound request must never reach.
///
/// Covers loopback, private (RFC1918), link-local, broadcast, multicast,
/// unspecified, documentation, CGNAT (RFC 6598) and the `0.0.0.0/8` reserved
/// block for IPv4; loopback, unspecified, multicast, unique-local (`fc00::/7`),
/// link-local (`fe80::/10`) and embedded IPv4-mapped addresses for IPv6.
pub fn is_blocked_destination(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_unspecified()
                || v4.is_documentation()
                // CGNAT 100.64.0.0/10 (RFC 6598).
                || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xc0) == 0x40)
                // 0.0.0.0/8 reserved.
                || v4.octets()[0] == 0
                // 169.254/16 link-local already covered, but explicit.
                || (v4.octets()[0] == 169 && v4.octets()[1] == 254)
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                // Unique-local fc00::/7.
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                // Link-local fe80::/10.
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                // IPv4-mapped (::ffff:0:0/96): defer to the v4 check by
                // re-mapping the embedded address.
                || v6
                    .to_ipv4_mapped()
                    .map(|v4| is_blocked_destination(IpAddr::V4(v4)))
                    .unwrap_or(false)
        }
    }
}

/// Resolve `host:port` and verify every candidate address is acceptable,
/// returning the first verified socket address to pin the HTTP client to.
///
/// Fails closed: if any resolved address falls into a blocked range, the whole
/// host is rejected. A host that points partly into a blocked range is
/// suspicious enough that the public-looking record is refused too, rather than
/// silently falling back to it.
pub async fn resolve_and_pin(host: &str, port: u16) -> Result<SocketAddr, GuardError> {
    let target = format!("{host}:{port}");
    let ips: Vec<SocketAddr> = tokio::net::lookup_host(&target)
        .await
        .map_err(|source| GuardError::Resolve {
            host: host.to_string(),
            source,
        })?
        .collect();
    if ips.is_empty() {
        return Err(GuardError::NoAddresses(host.to_string()));
    }
    for sa in &ips {
        if is_blocked_destination(sa.ip()) {
            return Err(GuardError::Blocked {
                ip: sa.ip(),
                host: host.to_string(),
            });
        }
    }
    Ok(ips[0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn v4(s: &str) -> IpAddr {
        IpAddr::V4(s.parse::<Ipv4Addr>().unwrap())
    }
    fn v6(s: &str) -> IpAddr {
        IpAddr::V6(s.parse::<Ipv6Addr>().unwrap())
    }

    #[test]
    fn blocks_the_private_and_local_ranges() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "192.168.0.1",
            "172.16.5.5",
            "169.254.1.1",
            "100.64.0.1", // CGNAT
            "0.0.0.0",
            "224.0.0.1", // multicast
            "255.255.255.255",
        ] {
            assert!(is_blocked_destination(v4(ip)), "{ip} must be blocked");
        }
        for ip in ["::1", "fc00::1", "fe80::1", "::ffff:127.0.0.1", "::ffff:10.0.0.1"] {
            assert!(is_blocked_destination(v6(ip)), "{ip} must be blocked");
        }
    }

    #[test]
    fn allows_public_addresses() {
        for ip in ["8.8.8.8", "1.1.1.1", "93.184.216.34"] {
            assert!(!is_blocked_destination(v4(ip)), "{ip} must be allowed");
        }
        assert!(!is_blocked_destination(v6("2606:4700:4700::1111")));
        // A public IPv4 embedded as IPv4-mapped stays allowed.
        assert!(!is_blocked_destination(v6("::ffff:8.8.8.8")));
    }

    #[tokio::test]
    async fn resolve_and_pin_blocks_loopback_literal() {
        let err = resolve_and_pin("127.0.0.1", 443).await.unwrap_err();
        assert!(matches!(err, GuardError::Blocked { .. }), "got {err:?}");
    }

    #[tokio::test]
    async fn resolve_and_pin_pins_public_literal() {
        let addr = resolve_and_pin("8.8.8.8", 443).await.expect("public literal pins");
        assert_eq!(addr, "8.8.8.8:443".parse::<SocketAddr>().unwrap());
    }

    #[test]
    fn cgnat_mask_is_precise_at_the_boundaries() {
        // 100.64.0.0/10 = 100.64.0.0 .. 100.127.255.255. The hand-rolled mask must
        // block exactly that and leave the neighbouring 100.x public space alone.
        for blocked in ["100.64.0.0", "100.127.255.255", "100.96.1.1"] {
            assert!(is_blocked_destination(v4(blocked)), "{blocked} is CGNAT");
        }
        for allowed in ["100.63.255.255", "100.128.0.0", "100.200.1.1", "99.64.0.1"] {
            assert!(!is_blocked_destination(v4(allowed)), "{allowed} is public");
        }
    }

    #[test]
    fn blocks_ipv4_documentation_ranges() {
        for ip in ["192.0.2.1", "198.51.100.1", "203.0.113.1"] {
            assert!(is_blocked_destination(v4(ip)), "{ip} is a documentation range");
        }
    }

    #[test]
    fn ipv6_ula_and_link_local_masks_are_precise() {
        // Unique-local fc00::/7 (fc00.. .fdff..) and link-local fe80::/10
        // (fe80.. .febf..) are bit-masked by hand; pin both edges.
        for blocked in ["fc00::1", "fdff::1", "fe80::1", "febf::1"] {
            assert!(is_blocked_destination(v6(blocked)), "{blocked} must be blocked");
        }
        // Just outside both ranges, and the fec0::/10 (deprecated site-local) gap
        // above link-local, are not caught by these masks.
        for allowed in ["fbff::1", "fec0::1", "2001:4860:4860::8888"] {
            assert!(!is_blocked_destination(v6(allowed)), "{allowed} must be allowed");
        }
    }

    #[test]
    fn ipv4_mapped_private_addresses_are_blocked() {
        // The classic SSRF bypass: a private v4 smuggled as an IPv4-mapped v6.
        for ip in ["::ffff:192.168.1.1", "::ffff:169.254.1.1", "::ffff:100.64.0.1"] {
            assert!(is_blocked_destination(v6(ip)), "{ip} must defer to the v4 check");
        }
    }
}
