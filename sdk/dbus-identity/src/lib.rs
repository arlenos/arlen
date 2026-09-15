// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Who is calling, over D-Bus, attested rather than claimed.
//!
//! A D-Bus service is not handed a connected socket the way a Unix-socket daemon
//! is, so it cannot read `SO_PEERPIDFD` and pin its peer. What it has is a sender
//! name, which the bus will turn into a pid. Every daemon in this tree that gates
//! a method on WHO is calling then had the same twenty lines: ask the bus for the
//! pid, read that pid's exe, resolve an app id, and guard against the pid being
//! recycled in between.
//!
//! **Five byte-identical copies of it, which is why this crate exists.** Not for
//! tidiness: the resolution has an open change in front of it (the broker tier
//! below), and five copies is how one gets missed. A caller-identity resolver that
//! is right in four daemons and stale in the fifth is worse than one that is
//! uniformly old.
//!
//! # The two tiers, and why the order is this way round
//!
//! **Tier 1, the launcher stamp.** `arlen-run` resolves an app's identity from the
//! root-owned registry BEFORE the process runs and registers it with the identity
//! broker against a pidfd. Asking the broker is therefore unforgeable by the
//! caller, and - the property that matters here - it touches no path at all:
//! `pidfd_open` is a pid-namespace operation and the broker's lookup is keyed on
//! the fd sent over `SCM_RIGHTS`. A daemon hardened with `ProtectSystem`,
//! `ProtectHome` and a private `/tmp` lives in its own mount namespace, where
//! reading another process's `/proc/<pid>/exe` is refused - measured on `capsuled`
//! on 14 September, where a fenced process could read exactly one exe link out of
//! 242 same-uid processes, its own. A daemon that identifies callers by exe alone
//! therefore refuses EVERYONE the moment it is packaged with its own hardening,
//! and the refusal reads as a permissions decision rather than as a mount table.
//!
//! **Tier 2, the exe read.** For a process no launcher stamped: a binary run from
//! a cargo target on a developer's machine, which is also not behind a fence. It
//! carries a pid-reuse guard - the process start time either side of the read -
//! because unlike the pidfd above it holds nothing open while it reads.
//!
//! Tier 1 is ADDITIVE. A miss is not a denial; it falls through, so a daemon that
//! resolves a caller today goes on resolving it. That is what lets the tier be
//! added to a working daemon without asking anybody to decide anything.

use arlen_permissions::identity::{app_id_from_pid, pid_start_time};
use arlen_permissions::stamped_identity::stamped_app_id_for_pid;
use zbus::message::Header;
use zbus::Connection;

/// The attested app id of the caller of the message `header` belongs to, and the
/// pid it was attested for.
///
/// The pid comes back alongside because some callers have to remember WHICH
/// process a capability was minted for, and re-asking later would name a different
/// process.
///
/// Every failure is an `Err` the caller MUST treat as a denial. The app id is
/// never a value the caller supplied.
///
/// # Errors
///
/// When the message carries no sender, the bus will not name the sender's pid,
/// neither tier can name the process, or the pid was recycled during the Tier-2
/// read.
pub async fn resolve_caller_with_pid(
    header: &Header<'_>,
    connection: &Connection,
) -> Result<(String, u32), String> {
    let sender = header.sender().ok_or_else(|| "no sender in message".to_string())?;
    let proxy = zbus::fdo::DBusProxy::new(connection)
        .await
        .map_err(|e| format!("DBusProxy: {e}"))?;
    let pid = proxy
        .get_connection_unix_process_id(sender.clone().into())
        .await
        .map_err(|e| format!("get caller pid: {e}"))?;

    // Tier 1. No `/proc` read, works inside a mount namespace, and pins the process
    // for the length of the lookup so it needs no reuse guard of its own.
    if let Some(app_id) = stamped_app_id_for_pid(pid) {
        return Ok((app_id, pid));
    }

    // Tier 2, guarded: the start time either side, so a pid recycled between the
    // bus attesting it and the exe read fails closed rather than naming whoever
    // inherited the number.
    let start_before = pid_start_time(pid).map_err(|e| format!("pid start time: {e}"))?;
    let app_id = app_id_from_pid(pid).map_err(|e| format!("resolve app id: {e}"))?;
    let start_after = pid_start_time(pid).map_err(|e| format!("pid start time: {e}"))?;
    if start_before != start_after {
        return Err("pid recycled during resolution".to_string());
    }
    Ok((app_id, pid))
}

/// [`resolve_caller_with_pid`] for the callers that only need the name.
///
/// # Errors
///
/// As [`resolve_caller_with_pid`].
pub async fn resolve_caller(
    header: &Header<'_>,
    connection: &Connection,
) -> Result<String, String> {
    resolve_caller_with_pid(header, connection).await.map(|(id, _)| id)
}
