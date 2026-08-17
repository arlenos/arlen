//! The undo service must not learn to ask whether the assistant is running.
//!
//! Its module doc states the rule and the reason: these operations used to live on
//! `org.arlen.AIAgent1`, which the AI engine registers only while `[ai] enabled` is
//! true, so switching the assistant off in Settings took a user's own file moves
//! out of the list and their undo with them. The records never depended on the
//! assistant. "Turn it off and still see and reverse everything it did" only holds
//! while nothing in this crate consults that switch.
//!
//! A rule stated in prose is one edit from being untrue, and the edit that breaks
//! it looks reasonable in review - reading the AI config to decide whether to show
//! AI-origin entries is the obvious thing to write. So the rule is checked.
//!
//! This lives in `tests/` rather than beside the code because the check searches
//! for the very strings it would have to contain, and a scanner that matches
//! itself is no scanner.

use std::path::Path;

/// Ways this crate could come to ask whether the assistant is on. Substrings, so
/// a rename around them still trips: what matters is that the AI switch is being
/// consulted at all, not the spelling of the call.
const ASKING: &[&str] = &[
    "ai.toml",
    "ai_enabled",
    "load_ai",
    "AiConfig",
    "ai_config",
    "[ai]",
    "AIAgent1",
];

/// Code with comments removed, so the module doc that EXPLAINS the rule does not
/// read as a violation of it.
fn code_only(source: &str) -> String {
    source
        .lines()
        .map(|line| {
            let t = line.trim_start();
            if t.starts_with("//") {
                return "";
            }
            match line.find("//") {
                Some(i) => &line[..i],
                None => line,
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_undo_service_never_consults_the_ai_switch() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut scanned = 0usize;
    let mut offences: Vec<String> = Vec::new();

    let entries = std::fs::read_dir(&src).expect("the crate has a src directory");
    for entry in entries {
        let path = entry.expect("readable dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("readable source file");
        let code = code_only(&source);
        scanned += 1;
        for needle in ASKING {
            if code.contains(needle) {
                offences.push(format!(
                    "{}: consults the AI switch via {needle:?}",
                    path.file_name().and_then(|f| f.to_str()).unwrap_or("?")
                ));
            }
        }
    }

    // Reading nothing is not passing: a moved or renamed source tree must fail
    // here rather than report a clean scan of an empty set.
    assert!(
        scanned >= 4,
        "expected to scan the crate's sources, saw {scanned} files"
    );
    assert!(
        offences.is_empty(),
        "the undo service must serve regardless of whether the assistant is on:\n  {}",
        offences.join("\n  ")
    );
}

/// The check must be able to fail, so the shape it looks for is exercised here.
#[test]
fn the_check_would_catch_a_reintroduced_ai_dependency() {
    let planted = "fn show(&self) -> bool {\n    load_ai_config().enabled\n}\n";
    let code = code_only(planted);
    assert!(
        ASKING.iter().any(|n| code.contains(n)),
        "a reintroduced dependency must trip the scan"
    );

    // And the module doc explaining the rule must NOT trip it, which is the
    // false positive that would otherwise get the check deleted.
    let doc = "//! It starts and serves regardless of `[ai] enabled`. Nothing in\n\
               //! this binary may learn to ask whether it is running.\n";
    let doc_code = code_only(doc);
    assert!(
        !ASKING.iter().any(|n| doc_code.contains(n)),
        "prose about the rule must not read as a breach of it"
    );
}
