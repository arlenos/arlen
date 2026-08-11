// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Turn a failed confined launch into something the user is told.
//!
//! A launch that `arlen-run` refuses exits within milliseconds and writes its
//! reason to stderr. The shell nulled that stderr and dropped the status, so the
//! refusal arrived as an icon that did nothing at all - the silent-stop shape,
//! on the one path whose whole job is to stop things.
//!
//! **The exit code alone cannot tell a refusal from an application.** `arlen-run`
//! propagates its child's status, so a program that runs and exits 65 is
//! indistinguishable by code from a profile that could not be loaded. What does
//! distinguish them is that every refusal path in the launcher prints a line
//! beginning with `arlen-run`, and the message this produces is that line rather
//! than an interpretation of it: the launcher already says why, in one sentence,
//! and re-deriving it here would be a second opinion that can drift.
//!
//! An application could print such a line itself. The cost of that is one
//! misleading toast about an app the user just started, which is not worth a
//! protocol to defend against; nothing is granted or refused on the strength of
//! this string.
//!
//! **Not every line the launcher prints is a refusal, and that invariant broke.**
//! Three of its diagnostics are things it continues past - an unregistered
//! identity stamp, unavailable stamp pipes, a missing per-launch cgroup - and
//! each of them says so in its own text ("app resolves via /proc", "launching
//! unstamped", "reaping falls back to bwrap"). They carried the same `arlen-run`
//! prefix as a refusal, and this function takes the FIRST matching line, so a
//! launch that failed later reported one of them as the reason: a sentence
//! stating the launch continued, offered as the explanation for why it did not.
//!
//! The stamp warning fires on every confined launch today, since no identity
//! broker answers in a user session, so this was not a corner. The launcher now
//! marks a continuing diagnostic `arlen-run: warning:` and this skips those, which
//! keeps the discrimination where the knowledge is: only the launcher knows which
//! of its own messages it survives.

/// The prefix every `arlen-run` diagnostic carries. Matched at the start of a
/// line, not anywhere in the output, so an application quoting the launcher's
/// name in passing does not become a refusal.
const MARKER: &str = "arlen-run";

/// A launcher line the launcher itself continued past.
///
/// Matched after the marker rather than anywhere in the line, so an application
/// whose own error mentions a warning is not mistaken for one. Kept narrow on
/// purpose: a refusal wrongly skipped is a silent stop, which is the failure this
/// whole module exists to remove, so only the launcher's explicit spelling counts.
fn is_warning(line: &str) -> bool {
    line[MARKER.len()..].trim_start().starts_with("warning:")
        || line[MARKER.len()..].trim_start().starts_with(": warning:")
}

/// What to tell the user about a launch that ended without starting, or `None`
/// when there is nothing to say.
///
/// `None` for a success, and `None` for an unconfined launch: an ordinary
/// application exits non-zero all the time, long after it started, and calling
/// that a failure to launch would make the first real refusal unremarkable.
pub fn refusal_message(app: &str, confined: bool, success: bool, stderr: &str) -> Option<String> {
    if success || !confined {
        return None;
    }
    let line = stderr
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with(MARKER) && !is_warning(l))?;
    // Drop the launcher's own prefix up to the first colon, keeping its sentence.
    // `arlen-run --landlock-exec:` is one of the spellings, so this cuts at the
    // colon rather than at a fixed width.
    let detail = line
        .split_once(':')
        .map(|(_, rest)| rest.trim())
        .unwrap_or(line);
    Some(format!("{app} did not start: {detail}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_launcher_refusal_becomes_a_sentence_in_the_launchers_own_words() {
        let out = refusal_message(
            "Clock",
            true,
            false,
            "arlen-run: profile not found for dev.arlen.clock\n",
        );
        assert_eq!(
            out.as_deref(),
            Some("Clock did not start: profile not found for dev.arlen.clock")
        );
    }

    // The case that was answered wrongly: the stamp warning is printed on every
    // confined launch today, so it was the first marker line in every failure and
    // therefore the reported reason - a sentence saying the app carried on.
    #[test]
    fn a_warning_the_launcher_continued_past_is_not_the_reason() {
        let out = refusal_message(
            "Files",
            true,
            false,
            "arlen-run: warning: identity stamp not registered (app resolves via /proc): connect: No such file\n\
             arlen-run: confinement setup for dev.arlen.files: bind source missing\n",
        );
        assert_eq!(
            out.as_deref(),
            Some("Files did not start: confinement setup for dev.arlen.files: bind source missing")
        );
    }

    // And the boundary: warnings alone mean the launch did not stop for a reason
    // this can name, so there is nothing to say rather than a warning to misreport.
    #[test]
    fn warnings_alone_produce_no_message() {
        let out = refusal_message(
            "Files",
            true,
            false,
            "arlen-run: warning: no per-launch cgroup (x); reaping falls back to bwrap\n",
        );
        assert_eq!(out, None);
    }

    #[test]
    fn the_other_spelling_of_the_prefix_is_recognised_too() {
        let out = refusal_message(
            "Clock",
            true,
            false,
            "arlen-run --landlock-exec: exec: No such file or directory (os error 2)",
        );
        assert_eq!(
            out.as_deref(),
            Some("Clock did not start: exec: No such file or directory (os error 2)")
        );
    }

    /// The case this exists to avoid saying anything about: an application that
    /// ran, did its work and exited non-zero hours later.
    #[test]
    fn an_applications_own_nonzero_exit_is_not_reported_as_a_refusal() {
        assert_eq!(
            refusal_message("Editor", true, false, "syntax error\n"),
            None
        );
        assert_eq!(refusal_message("Editor", false, false, ""), None);
    }

    #[test]
    fn a_launch_that_worked_says_nothing() {
        assert_eq!(
            refusal_message("Clock", true, true, "arlen-run: noise"),
            None
        );
    }

    /// The marker has to open the line. An application printing the launcher's
    /// name mid-sentence is not the launcher refusing.
    #[test]
    fn the_marker_is_only_a_marker_at_the_start_of_a_line() {
        assert_eq!(
            refusal_message("Editor", true, false, "could not reach arlen-run: busy"),
            None
        );
    }
}
