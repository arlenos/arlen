// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The event-bus consumer registration handshake, in one place.
//!
//! The bus reads **three** newline-terminated lines from a new consumer - id,
//! comma-separated type patterns, uid filter - and blocks on the third. Sending
//! two leaves the registration incomplete: the bus never reaches
//! `registry.register`, the consumer receives nothing, and nothing anywhere
//! reports a problem. Both ends are alive, the socket is connected, and no event
//! ever arrives.
//!
//! **That is not hypothetical.** The graph writer sent two lines for as long as
//! the uid filter had existed, so the raw-event to SQLite to promotion pipeline
//! was silently dead: the cache-invalidation consumer next door had been updated
//! to three and the writer had not. It was found by an integration test, months
//! later, and only because one was written.
//!
//! `os_sdk::event_consumer` owns this format for ordinary consumers and mints its
//! own ids. Neither site here can use it: the writer needs a FIXED id
//! (`graph-writer`, which the bus's own tests name) and its own three-tier
//! backpressure rather than the SDK's lossy channel. So the format is written out
//! twice in this daemon - and writing it out twice is exactly how the two drifted.
//! One function now, with the rule as a test rather than a comment.

/// The three-line registration a consumer sends the bus, terminator included.
///
/// Every line is terminated, including the last: the bus reads the third line
/// before it registers anything, so a missing final newline hangs the handshake
/// as surely as a missing line does.
pub fn registration(consumer_id: &str, patterns: &str, uid_filter: &str) -> String {
    format!("{consumer_id}\n{patterns}\n{uid_filter}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property the writer got wrong: three lines, each terminated.
    #[test]
    fn a_registration_is_three_terminated_lines() {
        let r = registration("graph-writer", "*", "*");
        assert!(r.ends_with('\n'), "the last line must be terminated too: {r:?}");
        assert_eq!(r.matches('\n').count(), 3, "the bus blocks on the third line: {r:?}");
        assert_eq!(r.lines().collect::<Vec<_>>(), ["graph-writer", "*", "*"]);
    }

    /// The other caller in this daemon, whose shape is the one the writer lacked.
    #[test]
    fn a_pattern_list_and_a_numeric_uid_keep_the_same_shape() {
        let r = registration("knowledge-config", "permission.*,ai.*,schema.*", "1000");
        assert_eq!(r.matches('\n').count(), 3);
        assert_eq!(
            r.lines().collect::<Vec<_>>(),
            ["knowledge-config", "permission.*,ai.*,schema.*", "1000"]
        );
    }
}
