// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! With the assistant off, its past actions stay visible and reversible.
//!
//! That property is currently asserted in three doc comments and one unit-file
//! comment, and nowhere else. Tonight's recurring finding was intent that was
//! written down and then not carried out - a flag exported, imported and never
//! rendered; a caveat argued for in a doc comment four lines above the branch
//! that ignored it. A property held only in prose is the same shape, so this is
//! the executable half.
//!
//! What it checks: no module in this crate reads the assistant's switch or talks
//! to its bus name. The undo service reads the signer's log and the audit
//! ledger, both of which exist whether or not the engine is running, so any
//! appearance of `[ai] enabled`, `ai.toml` or `org.arlen.AI` in code here would
//! be a new coupling - which is exactly how the coupling got there the first
//! time, before it was removed.
//!
//! What it does NOT check, and the second one is the bigger gap:
//!
//!   * The unit file. `arlen-undod.service` carries no `After=` on the engine
//!     and says why, but this test reads Rust only. A deployment can break this
//!     property without a line of code changing - that shape has bitten this
//!     project already, in the permission helper's `ReadWritePaths`.
//!   * That the service actually answers with the engine stopped. That needs
//!     both daemons and a bus, and belongs to an integration run rather than to
//!     `cargo test`. This narrows the ways the property can be lost silently; it
//!     does not observe the property holding.
//!
//! Shown to fail before being trusted: adding `let _ = "ai.toml";` to
//! `undo_history.rs` makes it fail, naming that file.

use std::fs;
use std::path::Path;

/// Tokens that would mean this crate had started depending on the assistant.
const COUPLINGS: &[&str] = &["ai.toml", "org.arlen.AI", "ai_enabled", "AiEngine"];

/// Strip `//` line comments, so the doc comments that explain this very
/// invariant do not trip it. A scanner that cannot tell a comment from code
/// reports the explanation as the violation - the same mistake as reading a map
/// key as a label.
fn code_only(src: &str) -> String {
    src.lines()
        .map(|line| match line.find("//") {
            Some(i) => &line[..i],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_undo_path_never_consults_the_assistants_switch() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut findings = Vec::new();
    let mut files = 0;

    for entry in fs::read_dir(&src).expect("the crate has a src directory") {
        let path = entry.expect("readable entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        files += 1;
        let text = code_only(&fs::read_to_string(&path).expect("readable module"));
        for token in COUPLINGS {
            if text.contains(token) {
                findings.push(format!(
                    "{}: mentions `{token}` outside a comment",
                    path.file_name().unwrap().to_string_lossy()
                ));
            }
        }
    }

    assert!(files > 0, "no modules scanned: the crate layout moved and this test did not");
    assert!(
        findings.is_empty(),
        "the undo service started depending on the assistant, which breaks the one \
         property it exists to keep - that switching the assistant off does not take \
         its past actions out of reach:\n  {}",
        findings.join("\n  ")
    );
}
