#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""A file that reads graph time must not carry a millisecond duration.

The graph stores every time field in EPOCH MICROSECONDS: `Event.timestamp` is
the envelope value promotion writes verbatim, `File.last_accessed` and
`Project.created_at` come from the same source, and the daemon's own reader
converts with `micros_to_dt`. A component that mints its own clock in
milliseconds and subtracts a stored value gets a number about 1.8e15 in the
wrong direction, and every one of these sites then wraps that in `.max(0)` for
clock skew, which turns the mistake into a plausible zero instead of a negative
number somebody would notice.

FOUND FOUR OF THESE IN ONE SWEEP on 26 August, none of which any test caught:

  - the quick-settings KG tile filtered on a millisecond cutoff, so its WHERE
    matched every event in the graph, and then divided every row out to a day a
    thousandfold past today and dropped it. It drew eight empty buckets and said
    "nothing has been recorded yet" as a fact.
  - the Waypointer's file plugin gave every file the full recency bump and
    described every one of them as accessed "just now".
  - its project plugin scored every project at the maximum, so the thirty-day
    falloff never ran and a project never opened ranked below one last touched
    two years ago.
  - (a fifth of the same family, not this rule's shape: the kernel layer stamped
    events with `bpf_ktime_get_ns`, boot-relative nanoseconds, in a field that
    means epoch micros.)

WHY THIS SHAPE. All three of the millisecond ones carried a day or week constant
in milliseconds sitting a few lines from the graph field: `const DAY_MS: i64 =
86_400_000`, `const WEEK_MS: i64 = 7 * 86_400_000`. That constant is the tell,
and it is a literal rather than a dataflow question, so it can be checked
exactly. The acknowledgement table below is EMPTY: on a correct tree no file
that names a graph time field carries one.

WHAT IT DOES NOT CATCH. A millisecond clock with no duration constant beside it
(`relative_time` divided a diff by 1000 with no day literal anywhere). That one
lived in a file this rule already flags for its week constant, so file
granularity covers it there, but it would not be caught alone. A `Duration`
millisecond value is not flagged at all: `from_millis` is the right unit for a
timeout or a sleep and appears in sixteen files that also read graph time, so
flagging it would be noise rather than a check.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
# An optional root so the control can point the rule at a built tree. The stale
# acknowledgement check below is scoped to the real repo: against a fixture,
# every acknowledged path is legitimately absent.
ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else REPO_ROOT

TREES = ("apps", "daemons", "sdk", "contracts")

# The graph's time columns, as they are written in a Cypher string or read off a
# row. `e.timestamp` rather than bare `timestamp` because the latter names a
# hundred unrelated things.
GRAPH_TIME_FIELDS = (
    "last_accessed",
    "created_at",
    "valid_at",
    "invalid_at",
    "expired_at",
    "issued_at",
    "last_exercised_at",
    "e.timestamp",
)

# Day, hour and week in milliseconds, in the spellings Rust accepts. The word
# boundary matters: `86_400_000_000` is the CORRECT microsecond day and must not
# match the millisecond one inside it.
MILLISECOND_DURATIONS = re.compile(
    r"\b(?:86_400_000|86400000|3_600_000|3600000|604_800_000|604800000"
    r"|86_400\s*\*\s*1_?000|24\s*\*\s*60\s*\*\s*60\s*\*\s*1_?000)\b"
)

# path -> why a millisecond duration is right there anyway. MAY SHRINK, MAY NOT
# GROW: a new entry is a component that started comparing two different clocks.
ACKNOWLEDGED: dict[str, str] = {}


def main() -> int:
    scanned = 0
    findings: list[str] = []

    for tree in TREES:
        base = ROOT / tree
        if not base.is_dir():
            continue
        for path in sorted(base.rglob("*.rs")):
            if "/target/" in str(path):
                continue
            try:
                text = path.read_text(encoding="utf-8", errors="replace")
            except OSError:
                continue
            if not any(f in text for f in GRAPH_TIME_FIELDS):
                continue
            scanned += 1
            rel = str(path.relative_to(ROOT))
            hits = [
                (i, line.strip())
                for i, line in enumerate(text.splitlines(), 1)
                if MILLISECOND_DURATIONS.search(line)
            ]
            if not hits:
                continue
            if rel in ACKNOWLEDGED:
                continue
            for line_no, line in hits[:3]:
                findings.append(
                    f"{rel}:{line_no}: a millisecond duration in a file that reads "
                    f"graph time, which is microseconds: `{line}`"
                )

    stale = (
        sorted(k for k in ACKNOWLEDGED if not (ROOT / k).exists())
        if ROOT == REPO_ROOT
        else []
    )

    if scanned == 0:
        print(
            "NOTHING WAS READ: no source file names a graph time field, so the "
            "rule matched nothing to check",
            file=sys.stderr,
        )
        return 2

    print(f"{scanned} file(s) that read graph time checked for a millisecond duration")

    if stale:
        print("\nacknowledgements naming files that no longer exist:\n")
        for k in stale:
            print(f"  - {k}")
        return 1

    if findings:
        print("\ntwo clocks in one subtraction:\n")
        for f in findings:
            print(f"  - {f}")
        print(
            "\nThe stored value is epoch microseconds. Mint the local clock with "
            "`timestamp_micros()` and scale the constant, or, if the duration "
            "never meets a stored value, keep it away from this file."
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
