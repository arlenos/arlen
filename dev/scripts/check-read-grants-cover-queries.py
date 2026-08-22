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

**Three query shapes it cannot see, measured on 11 Aug rather than guessed.** The
scan finds a binding as `(name:Label` and a read as `name.field`, both inside one
string literal, so these go past it entirely:

    (f:`File`)             a quoted label - the binding is not recognised, so
                           every field read off `f` is invisible
    MATCH (f) WHERE f:File the label arrives in a predicate, not the pattern
    "MATCH (f:File) " + q  a query assembled from more than one literal

Each one makes the gate QUIETER, never louder, which is the direction chosen
above. None of them occurs today: 30 `MATCH` lines across the seven app trees,
none quoted, none label-in-WHERE, none concatenated. So this is a statement about
what a future query could hide, not a defect being carried - **and the reason it
is written down is that a pattern is wrong in ways its own corpus cannot show.**
The sibling comment-path gate shipped with a boundary bug that its `.rs` corpus
had no string to expose, found only by running it over TypeScript.

Apps AND the daemons that have a profile. It was apps only, on the ground that
`code-indexer`, `modulesd`, `knowledge-mcp` and the AI engine "have no profile at
all" - and that stopped being true: `code-indexer.toml`, `modulesd.toml` and
`ai-agent.toml` are all in that directory now. The old text promised "it will
cover them the day they have one", and the day arrived without the glob noticing,
which is how a check quietly stops covering what it says it covers. The knowledge
daemon still owns the graph outright and has no profile to check.

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


def rust_files(paths):
    """Every `.rs` under `paths`, which may name directories or files."""
    for p in paths:
        if p.is_file():
            yield p
        else:
            yield from p.rglob("*.rs")


def fields_read(paths):
    """`(Label, field)` pairs the app's queries read, from query text only."""
    out = set()
    for rs in rust_files(paths):
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


#: Where a profile's id keeps its source, for the ids that are not `apps/<name>`.
#:
#: A LIST, and per-binary where a crate holds several. An app id is a ROLE
#: resolved from a path to an executable, so `ai-agent` is the AI engine daemon,
#: and `timeline` is ONE binary of the knowledge crate plus the module it calls
#: into - pointing that id at `src/bin` instead attributed a sibling diagnostic
#: binary's `MATCH (f:File ...) RETURN f.id` to the FUSE helper's profile, which
#: is a finding about a component that does not exist. An id with no entry and no
#: `apps/` directory is SKIPPED rather than guessed at.
DAEMON_DIRS = {
    "code-indexer": ["daemons/code-indexer"],
    "modulesd": ["daemons/modulesd"],
    "ai-agent": ["daemons/ai-engine-daemon"],
    "calendard": ["daemons/calendar"],
    "clockd": ["daemons/clock"],
    "powerd": ["daemons/power-daemon"],
    "auditd": ["daemons/audit-daemon"],
    "timeline": [
        "daemons/knowledge/src/bin/arlen-timeline.rs",
        "daemons/knowledge/src/fuse.rs",
    ],
}


def source_of(app_id):
    """The paths whose Cypher this profile answers for, or None."""
    if app_id in DAEMON_DIRS:
        paths = [REPO / p for p in DAEMON_DIRS[app_id]]
        present = [p for p in paths if p.exists()]
        return present or None
    d = REPO / "apps" / app_id.removeprefix("dev.arlen.")
    return [d] if d.is_dir() else None


def main():
    root = REPO / PROFILES
    # A profile carrying no grants at all is skipped deliberately, so zero
    # profiles CHECKED is a legitimate answer. Finding no profile FILE, or not
    # even the directory, is not: both are committed, so it means this ran
    # somewhere that is not the tree and the closing line would then vouch for
    # nothing.
    profiles = sorted(root.glob("*.toml")) if root.is_dir() else []
    if not profiles:
        print(f"NOTHING WAS READ: no app profile under {REPO / PROFILES}", file=sys.stderr)
        return 2

    problems = []
    checked = 0
    for profile in profiles:
        app_id = profile.stem
        app_dir = source_of(app_id)
        if app_dir is None:
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
