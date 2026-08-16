// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The core half of the shared bulk-rename contract. Its twin is
// `apps/files/src/lib/bulk-rename.test.ts`, which runs the SAME vectors against
// the TypeScript preview. The preview and the rename are two implementations of
// one rule set - the dialog computes names client-side so it can redraw per
// keystroke - and nothing checked they agreed until these vectors existed. A
// disagreement means the person approves one name and gets another.
//
// The core is authoritative: if a vector disagrees with this test, the vector is
// wrong unless the core is.

use arlen_file_browser_core::bulk_rename::{plan_rename, RenameRule};
use serde::Deserialize;

#[derive(Deserialize)]
struct Expected {
    from: String,
    to: String,
    conflict: String,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    names: Vec<String>,
    rule: RenameRule,
    expect: Vec<Expected>,
}

#[derive(Deserialize)]
struct Vectors {
    cases: Vec<Case>,
}

#[test]
fn the_core_satisfies_the_shared_rename_vectors() {
    let raw = include_str!("bulk-rename-cases.json");
    let vectors: Vectors = serde_json::from_str(raw).expect("the shared vectors parse");
    assert!(!vectors.cases.is_empty(), "vectors present");

    for case in &vectors.cases {
        let got = plan_rename(&case.names, &case.rule);
        assert_eq!(
            got.len(),
            case.expect.len(),
            "{}: one row per input name",
            case.name
        );
        for (row, want) in got.iter().zip(&case.expect) {
            assert_eq!(row.old, want.from, "{}: original name", case.name);
            assert_eq!(row.new, want.to, "{}: proposed name", case.name);
            let conflict = serde_json::to_string(&row.conflict).expect("conflict serialises");
            assert_eq!(
                conflict.trim_matches('"'),
                want.conflict,
                "{}: conflict for {}",
                case.name,
                want.from
            );
        }
    }
}
