//! Stamping the confined child's identity into the launcher-stamped identity
//! broker (the Tier-1 half of `stamped-identity-plan.md`).
//!
//! `arlen-run` holds the authenticated `--app-id` (resolved from the root
//! `IdentityRegistry` before launch), so it is the ONE process the broker trusts
//! to `Register` a stamp. The problem is WHICH pid to register: bwrap runs the app
//! under `--unshare-pid`, so the app has its OWN pid namespace and its host-visible
//! pid (the one a daemon reads via `SO_PEERPIDFD` at `accept`) differs from bwrap's
//! pid. bwrap reports that host pid via `--json-status-fd`: the first JSON document
//! it writes is `{ "child-pid": <host pid>, "mnt-namespace": ..., "pid-namespace":
//! ... }`.
//!
//! The full stamp handshake (wired into the spawn path in a following slice) is:
//!   1. Make two pipes: a json-status pipe and a block pipe.
//!   2. Add [`stamp_bwrap_args`] to the bwrap argv: `--json-status-fd <w_status>`
//!      (bwrap writes `child-pid` early, after the clone, before the app execs) and
//!      `--block-fd <r_block>` (bwrap waits for a byte on it before exec'ing the
//!      app).
//!   3. Spawn bwrap; in the parent, read the first json document from the status
//!      pipe and [`parse_child_pid`] it.
//!   4. `pidfd_open(child_pid)` -> register it at the broker with the app id
//!      (BEST-EFFORT: a broker outage or a register failure must NOT abort the
//!      launch; the app then simply resolves via /proc as `LegacyProc`, never a
//!      fabricated identity).
//!   5. Write one byte to the block pipe so bwrap unblocks and execs the app - so
//!      the stamp is recorded BEFORE the app can make its first daemon connection.
//!
//! This module holds the two PURE, format-critical pieces (the `child-pid` parse
//! and the bwrap arg assembly); the impure pipe/spawn/register wiring lands with
//! its integration test.

use std::os::fd::RawFd;

/// The bwrap flags that turn on the stamp handshake, for the given inherited fds:
/// `--json-status-fd <status_fd>` (bwrap writes the container status, incl.
/// `child-pid`, to it) and `--block-fd <block_fd>` (bwrap blocks reading it until
/// the launcher has registered the stamp, then execs the app). Prepended to the
/// confinement's own bwrap args.
pub fn stamp_bwrap_args(status_fd: RawFd, block_fd: RawFd) -> Vec<String> {
    vec![
        "--json-status-fd".to_string(),
        status_fd.to_string(),
        "--block-fd".to_string(),
        block_fd.to_string(),
    ]
}

/// Parse the host `child-pid` from bwrap's `--json-status-fd` output.
///
/// bwrap writes one or more JSON documents to the status fd; the FIRST carries
/// `"child-pid": <host pid>` (a later one carries `"exit-code"`). This scans for
/// the first `"child-pid"` key and reads the integer after its colon - a minimal
/// hand parse (no serde dep for one field), tolerant of surrounding whitespace and
/// the other keys in the document. Returns `None` if the key is absent, has no
/// integer, or the pid is `0` (never a real child pid; a bug guard so the caller
/// cannot `pidfd_open(0)`).
pub fn parse_child_pid(json_status: &str) -> Option<u32> {
    const KEY: &str = "\"child-pid\"";
    let after = &json_status[json_status.find(KEY)? + KEY.len()..];
    let after = after.trim_start().strip_prefix(':')?.trim_start();
    let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    match digits.parse::<u32>() {
        Ok(0) | Err(_) => None,
        Ok(pid) => Some(pid),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real bwrap 0.11 status stream: two documents, `child-pid` in the first.
    #[test]
    fn parses_child_pid_from_the_real_status_stream() {
        let stream = "{ \"child-pid\": 479882, \"mnt-namespace\": 4026534051, \"pid-namespace\": 4026534052 }\n{ \"exit-code\": 0 }\n";
        assert_eq!(parse_child_pid(stream), Some(479882));
    }

    /// Only the exit-code document (no child-pid) -> None, never a guessed pid.
    #[test]
    fn returns_none_without_a_child_pid() {
        assert_eq!(parse_child_pid("{ \"exit-code\": 0 }"), None);
        assert_eq!(parse_child_pid(""), None);
    }

    /// A child-pid key with no integer (truncated write) -> None.
    #[test]
    fn returns_none_on_a_malformed_child_pid() {
        assert_eq!(parse_child_pid("{ \"child-pid\": "), None);
        assert_eq!(parse_child_pid("{ \"child-pid\": abc }"), None);
    }

    /// A child-pid of 0 is refused (never a real pid; guards against pidfd_open(0)).
    #[test]
    fn refuses_a_zero_child_pid() {
        assert_eq!(parse_child_pid("{ \"child-pid\": 0 }"), None);
    }

    /// The stamp args carry the two flags with the fds rendered as decimals.
    #[test]
    fn stamp_args_carry_the_two_bwrap_flags() {
        assert_eq!(
            stamp_bwrap_args(7, 9),
            vec![
                "--json-status-fd".to_string(),
                "7".to_string(),
                "--block-fd".to_string(),
                "9".to_string(),
            ]
        );
    }
}
