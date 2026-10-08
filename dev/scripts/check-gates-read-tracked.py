#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Every gate that walks the tree reads what git knows about.

`tracked_walk` makes `Path.rglob`, `Path.glob` and `os.walk` skip what git
ignores, and it does so only in a gate that imports it. A gate written tomorrow
with a bare `rglob` and no import reads the image build's cache, a `target/`, a
`node_modules` - whatever is lying in the tree - and goes red over files nobody
here wrote, which is what happened to every commit on 8 October. This check is
the half that keeps the import from being forgotten.

Usage: check-gates-read-tracked.py [SCRIPTS_DIR]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
DIR = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else HERE

WALKS = re.compile(r'\.rglob\(|os\.walk\(|\.glob\(\s*"\*\*')
IMPORTS = re.compile(r"^\s*import tracked_walk\b", re.M)


def main() -> int:
    scripts = sorted(p for p in DIR.glob("*.py") if p.name != "tracked_walk.py")
    if not scripts:
        print(f"no gate scripts under {DIR}; refusing to pass on nothing")
        return 1
    bare = []
    for p in scripts:
        text = p.read_text(encoding="utf-8", errors="replace")
        if WALKS.search(text) and not IMPORTS.search(text):
            bare.append(p.name)
    if bare:
        for name in bare:
            print(
                f"{name} walks the tree and does not import tracked_walk, so it reads "
                "whatever is lying in the tree, ignored directories included. Add "
                "the guarded `import tracked_walk` the other gates carry, beside its imports."
            )
        return 1
    walking = sum(1 for p in scripts if WALKS.search(p.read_text(encoding="utf-8", errors="replace")))
    print(f"OK: {walking} gate(s) walk the tree, each through tracked_walk")
    return 0


if __name__ == "__main__":
    sys.exit(main())
