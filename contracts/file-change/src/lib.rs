//! The proposed/applied file-change contract - the body of the harness's diff
//! GateCard (surface 2 of `harness-file-refs-and-diffs-plan.md`), pinned by
//! arlen-ui (the diff/GateCard fusion, 29 June).
//!
//! One definition shared by the producer (the ai-agent executor's predict/plan)
//! and the consumer (the harness GateCard). The card consumes a
//! [`ChangeProposal`]: a one-line `title`, an optional reason, and the change as
//! a RAW UNIFIED DIFF string, which the card parses client-side (arlen-ui's
//! `parseUnifiedDiff` + `DiffView` in ui-kit). Emitting the raw diff string is
//! the pinned, easiest-for-the-daemon form; the structured per-file `DiffFile`
//! model is the card's own client-side parse, not a wire type.
//!
//! Proposed and done are the SAME payload (the plan's crux): the GateCard shows
//! it before approval and again as the done-receipt; the `done`/`auto`/`via`
//! framing lives on the frontend receipt envelope, not this payload.
//!
//! Producible today: a rename diff from the built `fs.move` action (a git-style
//! `rename from`/`rename to`, no content hunks). A content diff lands when
//! file-edit effects do; the daemon emits the raw unified diff for it then.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A proposed (or applied) file change the harness GateCard renders: a one-line
/// `title`, an optional `detail` (the predict-before-act reason), and the change
/// as a raw unified-diff `diff` string the card parses client-side. The same
/// payload is the proposed body and the done-receipt body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../bindings/")]
pub struct ChangeProposal {
    /// One-line action summary, e.g. `"Move report.pdf"` or `"Edit parser.rs"`.
    pub title: String,
    /// Why the change is proposed (the predict-before-act reason), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub detail: Option<String>,
    /// The change as a raw unified diff (git or plain), parsed by the card. A
    /// pure rename is a git `rename from`/`rename to` header with no hunks.
    pub diff: String,
}

impl ChangeProposal {
    /// A proposal for a pure file move/rename: a git-style rename diff (no content
    /// hunks), the form the built `fs.move` action produces. `from`/`to` are
    /// absolute paths; the card parses the `rename from`/`rename to` lines into a
    /// renamed `DiffFile`.
    pub fn rename(title: impl Into<String>, from: &str, to: &str) -> Self {
        Self {
            title: title.into(),
            detail: None,
            diff: format!(
                "diff --git a/{from} b/{to}\nsimilarity index 100%\nrename from {from}\nrename to {to}\n"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rename_proposal_carries_a_git_rename_diff_with_no_hunks() {
        let c = ChangeProposal::rename("Move report.pdf", "/dl/report.pdf", "/dl/2026/report.pdf");
        assert_eq!(c.title, "Move report.pdf");
        assert_eq!(c.detail, None);
        // The rename-from/to lines carry the clean absolute paths the card reads.
        assert!(c.diff.contains("rename from /dl/report.pdf"));
        assert!(c.diff.contains("rename to /dl/2026/report.pdf"));
        // A pure rename has no content hunks.
        assert!(!c.diff.contains("@@"));
        assert!(!c.diff.contains('+'));
    }

    #[test]
    fn it_round_trips_through_json_with_detail_optional() {
        let with_detail = ChangeProposal {
            title: "Edit parser.rs".to_string(),
            detail: Some("fix the off-by-one".to_string()),
            diff: "@@ -1 +1 @@\n-old\n+new\n".to_string(),
        };
        let back: ChangeProposal =
            serde_json::from_slice(&serde_json::to_vec(&with_detail).unwrap()).unwrap();
        assert_eq!(with_detail, back);

        // detail omitted on the wire when absent.
        let no_detail = ChangeProposal::rename("Move x", "/a/x", "/b/x");
        let json = String::from_utf8(serde_json::to_vec(&no_detail).unwrap()).unwrap();
        assert!(!json.contains("detail"));
    }
}
