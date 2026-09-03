#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Check that a build step looks for a binary some crate actually produces.

WHY THIS EXISTS, and the cost is measured rather than imagined. Every app phase
builds its frontend and backend, then finds the artefact by name:

    out=$(find "$CARGO_TARGET_DIR" -type f -path '*/release/arlen-pdf' ...)
    [ -n "$out" ] || { echo "!! arlen-pdf not found after build" >&2; exit 1; }

On 3 September that line cost a forty-minute image build. The pdf crate is
`arlen-pdf-app`, so cargo writes `release/arlen-pdf-app`; the step searched for
`arlen-pdf`, which is the name the app is INSTALLED under. `tauri build
--no-bundle` renames nothing - `productName` in tauri.conf.json names a bundle,
and that build makes none - so the search could never match. The phase failed
sixteen apps into the build, and the failure is only reachable by building.

The mismatch is invisible to every other check: the step is valid shell, the crate
compiles, the installed name is right everywhere else, and `check-binary-names`
asks a different question (that no two crates claim one name). Only the pair is
wrong, and the pair is exactly what this reads.

A search name must be a `[[bin]]` name or a package name somewhere in the tree.
That is deliberately loose: a step may install its artefact under a different name
afterwards, which is normal and fine. What is not fine is looking for something
nothing builds.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = (
    Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
)
STEPS = ROOT / "dev/mkosi/mkosi.build.d"

#: `-path '*/release/<name>'` and `-path '*/debug/<name>'`, the shape every phase
#: uses to find what cargo just wrote.
SEARCH = re.compile(r"-path\s+'\*/(?:release|debug)/([A-Za-z0-9_-]+)'")
#: A crate's own name, and any explicit `[[bin]] name`.
PACKAGE = re.compile(r'^\s*name\s*=\s*"([A-Za-z0-9_-]+)"', re.M)


def produced_names() -> set[str]:
    """Every binary name a crate in this tree could write."""
    names: set[str] = set()
    skip = {"target", "node_modules", ".git", "mkosi.builddir", "mkosi.cache", "mkosi.tools"}
    for manifest in ROOT.rglob("Cargo.toml"):
        if any(part in skip for part in manifest.parts):
            continue
        names.update(PACKAGE.findall(manifest.read_text(encoding="utf-8", errors="replace")))
    return names


def main() -> int:
    if not STEPS.is_dir():
        print(f"{STEPS} is missing; the image has no build steps")
        return 1

    produced = produced_names()
    if not produced:
        print("NOTHING WAS READ: no Cargo.toml named a package", file=sys.stderr)
        return 2

    problems: list[str] = []
    checked = 0
    for step in sorted(STEPS.glob("*")):
        if not step.is_file():
            continue
        for name in SEARCH.findall(step.read_text(encoding="utf-8", errors="replace")):
            checked += 1
            if name in produced:
                continue
            problems.append(
                f"{step.name} looks for a binary called `{name}` after building, and no "
                f"crate in this tree produces that name. The phase would fail - and it "
                f"fails DURING an image build, which is the expensive place to find out. "
                f"Search for the name cargo writes; installing it under another name "
                f"afterwards is fine and normal."
            )

    if problems:
        for p in problems:
            print(p)
        return 1

    print(
        f"{checked} artefact search(es) across the build steps, each naming a binary "
        f"some crate produces"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
