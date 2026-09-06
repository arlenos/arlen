#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Check that no tracked source file contains a NUL byte.

WHY, and it is a search problem rather than a runtime one. A NUL in a text file makes
`grep` classify the whole file as binary: it prints "binary file matches" instead of
the matching lines, and through any wrapper that keeps only matches, a search of that
file comes back EMPTY. Not "skipped" - empty. Every text search over the tree then
answers "no" for that file, quietly, for as long as the byte is there.

Measured on 7 September, and it cost a wrong conclusion before it cost anything else:
`grep -n "dir" sdk/ui-kit/src/lib/i18n/index.ts` returned nothing, so I concluded the
kit exports no `dir` store and was about to write that an app re-exports a name that
does not exist. `awk` found it three times over; the export had been on line 118 all
along. The byte had been there since August and nobody had noticed what it did.

BOTH INSTANCES WERE THE SAME GOOD IDEA SPELLED BADLY: a cache key joining two fields
with a separator that cannot occur in either, which is right, written as the raw byte
instead of `\\u0000`, which is not. The escape is the same character at runtime and
leaves the file readable by every tool.

So this refuses the byte, not the idea. If a file genuinely must carry binary, it does
not belong under a source extension.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

ROOT = (
    Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
)

# The extensions a person greps. A `.png` or a `.raw` is binary by nature and is not
# the subject here.
SOURCE = {
    ".ts", ".tsx", ".js", ".mjs", ".cjs", ".svelte", ".rs", ".py", ".sh", ".bash",
    ".toml", ".json", ".md", ".yml", ".yaml", ".css", ".html", ".proto", ".xml",
    ".service", ".conf", ".desktop", ".portal",
}


# Known, and each with the reason it is still here. An entry is a promise that
# somebody looked, not a place to put a file that annoys the check.
#
# FALSE WHEN: the file spells its separator `\u0000`, which is the same character
# and one keystroke away. `ai/pi-plugins/src/proof-store.ts` had the identical
# line and took that fix on 7 September; this one is `sdk/ui-kit`, which is
# arlen-ui's, so it is theirs to make and this list keeps CI honest meanwhile -
# green for what is known, red the moment a THIRD one appears.
ALLOWED: dict[str, str] = {
    "sdk/ui-kit/src/lib/i18n/index.ts": "arlen-ui's file; the cache-key separator on line 158",
}


def tracked(root: Path) -> list[Path]:
    """Every tracked file, or every file under root when it is not a checkout."""
    try:
        out = subprocess.run(
            ["git", "-C", str(root), "ls-files", "-z"],
            capture_output=True, text=True, check=True,
        ).stdout
        names = [n for n in out.split("\0") if n]
        if names:
            return [root / n for n in names]
    except (OSError, subprocess.CalledProcessError):
        pass
    return [p for p in root.rglob("*") if p.is_file()]


def offenders(root: Path) -> list[tuple[str, int]]:
    found = []
    for path in tracked(root):
        if path.suffix not in SOURCE or not path.is_file():
            continue
        try:
            data = path.read_bytes()
        except OSError:
            continue
        if b"\0" not in data:
            continue
        rel = str(path.relative_to(root))
        if rel in ALLOWED:
            continue
        line = data[: data.index(b"\0")].count(b"\n") + 1
        found.append((rel, line))
    return sorted(found)


def main() -> int:
    bad = offenders(ROOT)
    if not bad:
        # The count is part of the result: a clean run over a tree that carries
        # two known files says something different from a clean run over none.
        print(f"check-text-is-text.py       ok, {len(ALLOWED)} known and named")
        return 0
    print("A source file carries a NUL byte, so grep answers nothing for it:")
    for rel, line in bad:
        print(f"  {rel}:{line}")
    print()
    print("  Write the character as an escape - `\\u0000` in TypeScript, `\\0` in Rust")
    print("  - which is the same value and leaves the file searchable.")
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
