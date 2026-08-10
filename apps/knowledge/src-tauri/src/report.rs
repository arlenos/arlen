// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! One place where a failed call to the knowledge daemon says so out loud.
//!
//! Every command in this app returns its error to the webview, and that part is
//! right: the frontend answers a failed read with its fixture and says the read
//! failed, rather than showing absence as fact. But the webview console is not
//! reachable from outside the machine. On a booted system a refused read and an
//! empty graph look identical, and that is not a hypothetical - it cost hours of
//! screenshots that could not tell the two apart, while the app knew the answer
//! every time and had nowhere to say it.
//!
//! The channel works, which was checked rather than assumed. The session starts
//! this app through `systemd-cat`, so both its streams land in the journal, and
//! journal lines from that same channel appear in the boot's serial log with
//! kernel timestamps. Under the app's own tag there were zero lines, because the
//! app said nothing at all.
//!
//! What goes in the message: the command's name and the daemon's own words. What
//! does not: any row content. A call that failed returned none, and a call that
//! succeeded is not logged - this app reads a graph of the user's own activity,
//! and a log is a second place that data would then live.

/// Log a failed daemon call, then hand the message on unchanged.
///
/// The message is returned exactly as it arrived so the frontend keeps deciding
/// what the user sees; this only adds a listener outside the webview.
pub(crate) fn graph_call_failed(command: &str, error: impl std::fmt::Display) -> String {
    let message = error.to_string();
    log::error!("{command}: the graph call failed: {message}");
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The returned string is the daemon's, untouched. The frontend matches on
    /// what it gets back, so decorating the message here would change behaviour
    /// in a module whose whole job is to observe.
    #[test]
    fn the_message_is_handed_on_unchanged() {
        let out = graph_call_failed("knowledge_timeline", "permission denied");
        assert_eq!(out, "permission denied");
    }
}
