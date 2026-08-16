#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Check that no Cypher query carries a parenthesised WHERE group.

WHY. The knowledge daemon's read-scope pre-gate scans patterns with a token scanner, and every
`(` that no identifier precedes opens a NODE to it (daemon.rs:4234). That is deliberate and
must stay: Cypher allows a PATTERN as a predicate - `WHERE (f)-[:REL]->(p)` - so excusing a
group after WHERE would let an unlabelled pattern predicate through unscoped, which is a
fail-open in the gate that decides what a caller may read.

The cost lands on honest queries. The moment a predicate needs `WHERE (a OR b) AND c`, the
group reads as an unlabelled node and the WHOLE read is denied - with a message that, until
16 August, blamed the labels. The text-editor's lens hit exactly this: two labelled nodes, a
refusal about labels, and a panel that fell back to its sample.

That failure is SILENT where it matters. The app catches the error, shows a fixture or an empty
section, and nothing reaches a log anybody reads - the shape this whole class of bug takes. So
it is worth catching where it is cheap: at the commit, in the query text.

THE RULE binds a CALLER: a file that builds a graph pattern AND sends it through the SDK client
(`UnixGraphClient`), because only those cross the socket the gate sits on. No line in one of
those may open a parenthesised group right after `WHERE`. Write the predicate without it -
choose the branch in Rust, as `lens.rs::project_query` does.

Two things are deliberately out of scope, and both are why the rule keys on the client rather
than on `MATCH (` alone. The knowledge daemon's OWN queries never pass its gate - they go
straight to the graph thread - so its source is free to quote the refused shape, which its
message and its regression test both do. And SQL is not Cypher: the audit ledger is SQLite, and
its `WHERE (?1 IS NULL OR project_id = ?1)` is perfectly fine.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = (
    Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
)
SEARCH = ("apps", "daemons", "sdk", "ai", "dev")

# `WHERE (` with any spacing, which is the shape the scanner reads as a node.
GROUP = re.compile(r"\bWHERE\s*\(")
BUILDS_CYPHER = re.compile(r"MATCH\s*\(")
# The socket client is what makes a query a GATED read. Without it the file is
# either the daemon (which does not pass its own gate) or not talking to the
# graph at all.
GATED_CALLER = re.compile(r"\bUnixGraphClient\b")
# A comment is not a query. The lens that found this documents the shape.
COMMENT = re.compile(r"^\s*(//|#|\*|///|//!)")


def rust_files() -> list[Path]:
    out: list[Path] = []
    for top in SEARCH:
        base = ROOT / top
        if not base.is_dir():
            continue
        out.extend(
            p
            for p in base.rglob("*.rs")
            if "target" not in p.parts and "node_modules" not in p.parts
        )
    return sorted(out)


def main() -> int:
    files = rust_files()
    if not files:
        print("no Rust sources found; the layout moved and this check did not")
        return 1

    problems: list[str] = []
    scanned = 0

    for path in files:
        text = path.read_text(encoding="utf-8", errors="replace")
        if not BUILDS_CYPHER.search(text) or not GATED_CALLER.search(text):
            continue  # not a gated read: the daemon itself, SQL and prose are all out of scope
        scanned += 1
        for n, line in enumerate(text.splitlines(), start=1):
            if COMMENT.match(line):
                continue
            if GROUP.search(line):
                problems.append(
                    f"{path.relative_to(ROOT)}:{n}: a parenthesised WHERE group reads as an "
                    f"unlabelled node to the read gate, and the whole query is denied"
                )

    if problems:
        print("\nCypher queries the read gate will refuse:\n")
        for p in problems:
            print(f"  - {p}")
        print(
            "\nWrite the predicate without the group - decide the branch in Rust, the way "
            "apps/text-editor/src-tauri/src/lens.rs::project_query does. The gate cannot "
            "excuse the parenthesis: a pattern is legal in WHERE, so that would be a fail-open."
        )
        return 1

    if not scanned:
        print("no gated graph readers found; that is not plausible")
        return 1

    print(f"{scanned} gated graph reader(s): no parenthesised WHERE group.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
