#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""An app's `[graph] read` grants should name the fields its queries read.

Checked by hand on 11 August across the four app profiles that hold grants, and
two of the four did not:

    desktop-shell   granted `system.File.id`; `recent_files.rs` returns path and
                    last_accessed and filters on last_accessed, and the Waypointer
                    file plugin also returns app_id.
    knowledge       granted four Event fields; the timeline returns a fifth
                    (`app_id`), and two queries filter on `Project.expired_at`.

Neither was breaking anything, and that is the point. Both apps are in the
first-party set, so `tier_for_app` makes them system-anchored and the read-scope
gate does not apply to them at all - the declaration can drift arbitrarily far
from the code and nothing complains. It only becomes a bug the day that anchoring
tightens, and then it is not an error: the query still runs and the column comes
back empty, so the symptom is a Recent Files list that is simply blank.

**One direction only, deliberately.** This reports a field a query READS that the
profile does not grant. It does NOT report a grant nothing seems to use, and the
asymmetry is not laziness:

  - a missed use makes the check quieter, which is a check that catches less;
  - a wrongly-reported unused grant invites deleting a grant that IS needed, and
    an app losing a field it reads is the failure this file exists to prevent.

I nearly made exactly that mistake by hand on `system.File.path` in the file
manager, on the strength of a regex that had not seen the multi-line queries at
`lib.rs:593` and `1729`. So the scan below reads inside query text only, and when
it cannot tell, it says nothing.

Apps only, and not because daemons do not query: `code-indexer`, `modulesd`,
`knowledge-mcp` and the AI engine all write Cypher, and the knowledge daemon owns
the graph outright. None of them has a profile at all, which is the open question
about whether daemons get profiles, not something this check can decide. It will
cover them the day they have one; until then a scope of `dev.arlen.*` is the whole
set that exists.

**Filter fields count, and the knowledge profile used to say the opposite.** Its
header held that a filter is not a returned field so the scope covers what is read
back, which is a coherent rule and not the one this check applies. What decided it
was reading how the scope is consumed: `readable_system_labels` (daemon.rs:793)
strips the field off every pattern and keeps the LABEL, so the field half is not
enforced on the read path at all - these lists are documentation today, and a
filter is the most useful thing in them, because it is exactly what a later
field-level gate would deny while the query still looks like it should work.
"""

import re
import sys
from pathlib import Path

REPO = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

PROFILES = "dev/mkosi/mkosi.extra/var/lib/arlen/permissions/1000"

# A grant line: "system.File.path".
GRANT = re.compile(r'"system\.([A-Za-z]+)\.([a-z_]+)"')

# A Cypher binding: `(f:File` / `(p:Project {` - the label a short name stands for.
BINDING = re.compile(r"\(\s*([a-z][a-z0-9_]*)\s*:\s*([A-Z][A-Za-z]*)")

# A field read off a bound name: `f.path`, `p.expired_at`. Bare `\b` on both ends
# so `f.path_of` is not read as `f.path`.
FIELD = re.compile(r"\b([a-z][a-z0-9_]*)\.([a-z_]+)\b")

# name -> why a used-but-ungranted field is carried rather than fixed.
KNOWN: dict[str, str] = {}


# A Rust string literal, newlines included: a long query is wrapped with `\`
# continuations, which are escapes INSIDE the literal, so one literal spans the
# whole query. Bounding on the literal is the point - see below.
STRING = re.compile(r'"(?:[^"\\]|\\.)*"', re.S)


def query_windows(text):
    """The stretches of a source file that are Cypher, not Rust.

    String literals containing `MATCH (`, and nothing else. The first version
    anchored on `MATCH (` and ran to the next `;` or 600 characters, which walked
    straight out of the query and into the code below it: in the file manager it
    reached the sort comparators in `core/src/lib.rs` and reported that a query
    read `system.App.name`, on the strength of `a.name.to_lowercase()`. Rust and
    Cypher share the `x.field` shape, so a window that overshoots does not look
    wrong - it looks like a finding, and the fix it invites is granting a field
    nothing reads.
    """
    for m in STRING.finditer(text):
        window = m.group(0)
        if "MATCH (" in window or "MATCH(" in window:
            yield window


def fields_read(app_dir):
    """`(Label, field)` pairs the app's queries read, from query text only."""
    out = set()
    for rs in app_dir.rglob("*.rs"):
        if "/target/" in str(rs):
            continue
        try:
            text = rs.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        for window in query_windows(text):
            bound = {name: label for name, label in BINDING.findall(window)}
            for name, field in FIELD.findall(window):
                if name in bound:
                    out.add((bound[name], field))
    return out


def main():
    root = REPO / PROFILES
    if not root.is_dir():
        print(f"no {PROFILES}; nothing to check")
        return 0

    problems = []
    checked = 0
    for profile in sorted(root.glob("dev.arlen.*.toml")):
        app_id = profile.stem
        app_dir = REPO / "apps" / app_id.removeprefix("dev.arlen.")
        if not app_dir.is_dir():
            continue
        text = profile.read_text(encoding="utf-8")
        granted = set(GRANT.findall(text))
        if not granted:
            # No grants at all is a deliberate state for an app that reads the
            # graph through an allowlisted op instead (meetings does), and this
            # check has no way to tell that apart from an omission. The profile
            # gate covers "has a profile"; this one covers "the grants it has".
            continue
        checked += 1
        for label, field in sorted(fields_read(app_dir) - granted):
            if app_id in KNOWN:
                continue
            problems.append(
                f"{app_id}: a query reads system.{label}.{field}, which the profile does not grant"
            )

    if problems:
        print("read grants do not cover the queries:")
        for p in problems:
            print(f"  {p}")
        print("  Add the field, or list the app in KNOWN with the reason it stays out.")
        print("  It is inert while the app is system-anchored; the breakage arrives")
        print("  with the tightening, as a column that comes back empty.")
        return 1

    print(f"OK: {checked} profile(s) with grants; every field their queries read is granted")
    return 0


if __name__ == "__main__":
    sys.exit(main())
