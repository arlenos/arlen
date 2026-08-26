#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""A token the backend sends must be one the frontend's union names.

`check-invoke-shape` compares the two sides' field NAMES. This compares the
VALUES, for the one shape where both sides state them: a frontend field typed as
a union of string literals, fed by a Rust struct of the same name.

FOUND ON 26 AUGUST, which is why this exists. `stitch_file_provenance` built each
provenance step's `relation` as English prose - `"Part of"`, `"Last opened by"`,
`"Also opened by"` - and the halo compares against a token
(`s.relation === "lastOpenedBy" ? ... : partOf`). The comparison was false for
every step, so a file the graph knew was last opened by an app was worded as
being PART OF that app. Right actor, wrong relation, a whole grammatical sentence
in both languages, and neither side's tests caught it: the Rust test asserted
`Some("Part of")`, agreeing with the producer about a value the consumer never
matched.

WHY IT PAIRS BY STRUCT NAME. Pairing by field name would be wrong more often than
right - `kind` alone appears as six unrelated unions in this tree
(applet/tray, entry/separator, local/cloud, monitor/window/region,
standard/created/guest, static/live). The same-name mirror is the house
convention and it is exact: `ProvenanceStep` in `apps/files/src-tauri` and
`ProvenanceStep` in `apps/files/src` are the two halves of one thing.

WHY IT READS ONLY INSIDE THE STRUCT'S OWN BODIES. The first cut scanned the crate
for `field: "literal"` anywhere and reported three findings, all three false: a
`status: "active"` on a Project and a `kind: "local"` on an AI provider, matched
to an unrelated union because they happened to share a field name. Literals are
now collected only from inside a `Name { ... }` construction, which took the
false positives to zero.

WHAT IT DOES NOT SEE, deliberately, since a check that guesses is worse than one
that is quiet:

  * a field assigned from a variable or a function, which is most of them. No
    literal found means nothing is said.
  * a union nested one type deep (a `Chain` whose `steps` are `Step[]` is paired
    by `Step`'s own name, not through the chain).
  * a value built with `format!` or a `match`, which is not a literal assignment.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else REPO_ROOT

# `field: "a" | "b";` and `field?: "a" | "b";`, lowercase-initial literals only,
# so a union of type names is not read as a union of tokens.
UNION = re.compile(
    r'^\s*([a-zA-Z_][a-zA-Z0-9_]*)\??\s*:\s*'
    r'("(?:[a-z][a-zA-Z0-9-]*)"(?:\s*\|\s*"(?:[a-z][a-zA-Z0-9-]*)")+)\s*;'
)


def balanced(text: str, open_at: int) -> str:
    """The body between the brace at `open_at` and its match."""
    depth, i = 0, open_at
    while i < len(text):
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                return text[open_at + 1 : i]
        i += 1
    return ""


def main() -> int:
    compared = 0
    findings: list[str] = []

    apps = ROOT / "apps"
    if not apps.is_dir():
        print(f"NOTHING WAS READ: no {apps}", file=sys.stderr)
        return 2

    for app_dir in sorted(apps.iterdir()):
        src_tauri = app_dir / "src-tauri" / "src"
        src = app_dir / "src"
        if not src_tauri.is_dir() or not src.is_dir():
            continue
        rust = "".join(
            p.read_text(encoding="utf-8", errors="replace")
            for p in sorted(src_tauri.rglob("*.rs"))
        )
        structs = set(re.findall(r"\bstruct\s+([A-Z][A-Za-z0-9]*)", rust))
        if not structs:
            continue

        fronts = [p for p in sorted(src.rglob("*.ts")) if "node_modules" not in p.parts]
        fronts += [p for p in sorted(src.rglob("*.svelte")) if "node_modules" not in p.parts]
        for path in fronts:
            text = path.read_text(encoding="utf-8", errors="replace")
            for m in re.finditer(r"(?:export\s+)?interface\s+([A-Z][A-Za-z0-9]*)\s*\{", text):
                name = m.group(1)
                if name not in structs:
                    continue
                for line in balanced(text, m.end() - 1).splitlines():
                    um = UNION.match(line)
                    if not um:
                        continue
                    field = um.group(1)
                    union = set(re.findall(r'"([^"]+)"', um.group(2)))
                    lits: set[str] = set()
                    for cm in re.finditer(r"\b" + re.escape(name) + r"\s*\{", rust):
                        body = balanced(rust, cm.end() - 1)
                        lits |= set(
                            re.findall(
                                r"\b" + re.escape(field) + r'\s*:\s*(?:Some\()?"([^"]+)"',
                                body,
                            )
                        )
                    if not lits:
                        continue
                    compared += 1
                    for bad in sorted(lits - union):
                        findings.append(
                            f"{app_dir.name}/{name}.{field}: the backend sends "
                            f"`{bad}`, which the union does not name "
                            f"({', '.join(sorted(union))}). The window compares "
                            f"against these, so this value takes whichever branch "
                            f"is the fallback."
                        )

    if compared == 0:
        print(
            "NOTHING WAS READ: no frontend union field is fed by a literal in a "
            "same-named Rust struct, so this compared nothing",
            file=sys.stderr,
        )
        return 2

    print(f"{compared} union field(s) compared against the literals their struct sends")
    if findings:
        print("\na token the window cannot match:\n")
        for f in findings:
            print(f"  - {f}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
