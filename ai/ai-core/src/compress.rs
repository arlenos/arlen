//! Lossless-in-meaning context compression for the agent read-path (RTK-style).
//!
//! Tool outputs and KG slices fed to a local model are often padded with redundancy the model
//! does not need - a log line repeated hundreds of times, a heartbeat, a retry storm. This
//! module collapses runs of byte-identical consecutive lines into the line plus a count marker,
//! which preserves the meaning (the model still sees the content and how many times it
//! occurred) while cutting tokens, easing the local-model context ceiling.
//!
//! Two safety rails keep it honest:
//! - **Lossless in meaning.** The only transform is collapsing an *exactly-repeated* line; the
//!   line's bytes are preserved and the repetition count is stated. Nothing unique is dropped.
//!   The collapse threshold is gated by the detected content kind, so structured text (a diff,
//!   code) is only collapsed on a long run, never on a few incidental duplicates.
//! - **Revert if it grows.** If the result is not smaller than the input, the original is
//!   returned unchanged. A misdetection or an unhelpful filter can never inflate the context.
//!
//! Richer per-kind filters (git-diff context trimming, grep path-prefix folding) are deferred:
//! each must be proven lossless for its kind before it ships, and several tempting ones are
//! not (stripping trailing whitespace hides a whitespace-only diff; squeezing columns corrupts
//! a filename containing double spaces). The safe, content-agnostic line-collapse ships now.

/// The detected shape of a block of tool output / KG data, used to pick the collapse threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentKind {
    /// A unified diff (`diff --git`, `@@` hunks). Structured - collapse conservatively.
    GitDiff,
    /// A log or command dump (timestamps, level markers). Repetition here is usually noise -
    /// collapse aggressively.
    LogDump,
    /// Anything else (grep output, a file listing, a KG slice, prose). Collapse conservatively.
    Unknown,
}

/// Collapse a run of this many identical lines in a log dump.
const LOG_THRESHOLD: usize = 3;
/// Collapse a run of this many identical lines in structured/unknown content (higher, so a few
/// incidental duplicates - e.g. repeated `}` lines in code - are never collapsed).
const CONSERVATIVE_THRESHOLD: usize = 6;

/// Classify a block from its first ~1KB. Cheap, allocation-light heuristics; only `LogDump`
/// changes behaviour today (a lower collapse threshold), the rest share the conservative one.
pub fn detect_kind(text: &str) -> ContentKind {
    let head_end = text.char_indices().nth(1024).map(|(i, _)| i).unwrap_or(text.len());
    let head = &text[..head_end];
    if head.contains("diff --git") || head.contains("\n@@ ") || head.starts_with("@@ ") {
        return ContentKind::GitDiff;
    }
    let log_lines = head.lines().filter(|l| looks_like_log(l)).count();
    if log_lines >= 3 {
        return ContentKind::LogDump;
    }
    ContentKind::Unknown
}

/// Whether a line looks like a log entry: a level marker or an `HH:MM:SS` clock time.
fn looks_like_log(line: &str) -> bool {
    line.contains("ERROR")
        || line.contains("WARN")
        || line.contains("INFO")
        || line.contains("DEBUG")
        || line.contains("TRACE")
        || has_clock_time(line)
}

/// Whether the line contains an `HH:MM:SS` clock time (digit digit `:` digit digit `:` digit
/// digit), a cheap timestamp signal.
fn has_clock_time(line: &str) -> bool {
    line.as_bytes().windows(8).any(|w| {
        w[0].is_ascii_digit()
            && w[1].is_ascii_digit()
            && w[2] == b':'
            && w[3].is_ascii_digit()
            && w[4].is_ascii_digit()
            && w[5] == b':'
            && w[6].is_ascii_digit()
            && w[7].is_ascii_digit()
    })
}

/// Losslessly (in meaning) compress a block for LLM context: collapse runs of identical lines,
/// with the threshold chosen by the detected kind, and return the original unchanged if the
/// result is not smaller (revert-if-grows). Empty input is returned as-is.
pub fn compress(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let threshold = match detect_kind(text) {
        ContentKind::LogDump => LOG_THRESHOLD,
        ContentKind::GitDiff | ContentKind::Unknown => CONSERVATIVE_THRESHOLD,
    };
    let collapsed = collapse_identical_runs(text, threshold);
    if collapsed.len() < text.len() {
        collapsed
    } else {
        text.to_string()
    }
}

/// Collapse each run of `threshold`-or-more consecutive byte-identical lines into the line once
/// plus a `[repeated N times]` marker. Shorter runs and unique lines pass through verbatim, so
/// the non-collapsed text is byte-preserved (`split('\n')` then `join('\n')` is the identity).
fn collapse_identical_runs(text: &str, threshold: usize) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let mut j = i + 1;
        while j < lines.len() && lines[j] == line {
            j += 1;
        }
        let run = j - i;
        if run >= threshold {
            out.push(line.to_string());
            out.push(format!("[repeated {run} times]"));
        } else {
            for _ in 0..run {
                out.push(line.to_string());
            }
        }
        i = j;
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_the_content_kind() {
        assert_eq!(detect_kind("diff --git a/x b/x\n@@ -1 +1 @@\n-a\n+b\n"), ContentKind::GitDiff);
        assert_eq!(
            detect_kind("12:00:01 INFO up\n12:00:02 WARN slow\n12:00:03 ERROR down\n"),
            ContentKind::LogDump
        );
        assert_eq!(detect_kind("just some prose\nwith two lines\n"), ContentKind::Unknown);
    }

    #[test]
    fn collapses_a_repetitive_log_losslessly() {
        // a retry storm: one line repeated far past the log threshold
        let mut log = String::from("12:00:00 INFO start\n");
        for _ in 0..50 {
            log.push_str("12:00:01 WARN connection retry\n");
        }
        log.push_str("12:00:59 INFO recovered\n");
        let out = compress(&log);
        assert!(out.len() < log.len(), "the storm should shrink");
        assert!(out.contains("12:00:01 WARN connection retry"), "the line content is preserved");
        assert!(out.contains("[repeated 50 times]"), "the count is stated");
        // the unique framing lines survive
        assert!(out.contains("12:00:00 INFO start"));
        assert!(out.contains("12:00:59 INFO recovered"));
    }

    #[test]
    fn leaves_structured_content_with_few_duplicates_untouched() {
        // code with a few identical closing-brace lines must NOT be collapsed (below the
        // conservative threshold), so structure is preserved.
        let code = "fn a() {\n    x();\n}\n}\n}\nfn b() {}\n";
        assert_eq!(compress(code), code, "3 identical lines is under the conservative threshold");
    }

    #[test]
    fn reverts_when_nothing_shrinks() {
        let prose = "a unique line\nanother unique line\nand a third\n";
        assert_eq!(compress(prose), prose);
        assert_eq!(compress(""), "");
    }

    #[test]
    fn a_long_run_in_unknown_content_still_collapses() {
        // even Unknown content collapses a run past the conservative threshold (lossless).
        let mut t = String::from("header\n");
        for _ in 0..10 {
            t.push_str("same\n");
        }
        let out = compress(&t);
        assert!(out.contains("same"));
        assert!(out.contains("[repeated 10 times]"));
        assert!(out.len() < t.len());
    }
}
