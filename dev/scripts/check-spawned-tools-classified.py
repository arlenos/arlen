#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Check that every external tool the tree spawns has been classified.

WHY. `check-spawned-binaries.py` covers our OWN binaries and deliberately stops
there, on the reasoning that which third-party packages an image installs is a build
decision a script should not hold an opinion about. That reasoning is right and it
left a silence: nobody had measured how many of those tools the image actually has.

Measured 20 August, by mounting the built image and looking: of 38 external binaries
the tree spawns, **23 are not on it**. Not a rounding error - `nmcli` (the whole
network popover), `pactl` and `wpctl` (audio), `wl-copy`/`wl-paste` (clipboard),
`rfkill` (airplane mode), `powerprofilesctl` (power profiles), `xdg-mime` and
`gtk-launch` (open-with and launching), `flatpak` (every Flatpak path), `git`
(forage's own `install git+URL`). Each is built, driven on a developer host where the
tool exists, and inert on the machine we ship.

So this check does NOT say which packages to install. It says every spawned tool must
be CLASSIFIED - shipped, or absent with the surface that stops working. A new
`Command::new("something")` for a tool nobody has thought about fails here, and the
absent list is a work item rather than a silence.

Re-measure with:

    guestfish --ro -a dev/mkosi/arlen.raw run : mount-ro /dev/sda2 / : sh '...'

Run: dev/scripts/check-spawned-tools-classified.py [root]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

TREES = ["apps", "daemons", "sdk", "ai", "forage", "store-backend"]
SPAWN = re.compile(r'Command::new\("([a-z][a-z0-9_.-]*)"')

#: What the image carries, READ from `runtime-deps.tsv` rather than kept here.
#:
#: These were two hand-kept tables until 21 August, and the tsv was the other
#: answer to the same question. They disagreed in three places: two apps this
#: file called absent were listed there as staged by the image build, which no
#: build step does, and `appstreamcli` was classified here and missing there
#: entirely because that check did not scan `forage`. Correcting the rows made
#: three real defects visible, so the disagreement was not cosmetic.
#:
#: One fact, one file. This check still asks its own question - is every spawned
#: tool classified at all - and `check-runtime-deps` asks the other direction,
#: that every classified tool is still spawned and its package named.
def classified(root: Path) -> tuple[set[str], dict[str, str]]:
    """`(shipped, absent)` from `runtime-deps.tsv`, keyed by tool."""
    shipped: set[str] = set()
    absent: dict[str, str] = {}
    for line in (root / "dev/scripts/runtime-deps.tsv").read_text().splitlines():
        if line.startswith("#") or not line.strip():
            continue
        fields = line.split("\t")
        if len(fields) < 4:
            continue
        tool, state = fields[0].strip(), fields[3].strip()
        note = fields[4].strip() if len(fields) > 4 else ""
        if state == "absent":
            absent[tool] = note or "no consequence recorded"
        else:
            shipped.add(tool)
    return shipped, absent


def spawns() -> dict[str, list[str]]:
    """Every external tool the tree spawns, with one file that spawns it."""
    found: dict[str, list[str]] = {}
    for tree in TREES:
        base = ROOT / tree
        if not base.is_dir():
            continue
        for path in sorted(base.rglob("*.rs")):
            if "target" in path.parts or "node_modules" in path.parts:
                continue
            text = path.read_text(encoding="utf-8", errors="replace")
            for tool in SPAWN.findall(text):
                found.setdefault(tool, []).append(str(path.relative_to(ROOT)))
    return found


def main() -> int:
    found = spawns()
    if not found:
        print("NOTHING WAS READ: no spawn sites found, so this checked nothing", file=sys.stderr)
        return 2

    SHIPPED, ABSENT = classified(ROOT)
    unclassified = {t: f for t, f in found.items() if t not in SHIPPED and t not in ABSENT}
    # STALENESS IS ONLY KNOWABLE OVER THE WHOLE TREE. On an arbitrary root - a
    # control's fixture, a subdirectory - "this entry is dead" and "this fixture
    # is small" look identical, and failing on the second would make the check
    # impossible to write a control for. Over the real repository the lists are
    # meant to shrink, and an entry nobody spawns any more is one to delete.
    own_repo = ROOT == Path(__file__).resolve().parents[2]
    stale = sorted((SHIPPED | ABSENT.keys()) - found.keys()) if own_repo else []

    problems = []
    for tool, files in sorted(unclassified.items()):
        problems.append(
            f"`{tool}` is spawned by {files[0]} and is in neither list.\n"
            f"    Classify it in dev/scripts/runtime-deps.tsv: on the image, or absent (with "
            f"runtime-deps.tsv with the surface that stops working). An unclassified spawn is a "
            f"that may already be inert on the machine we ship."
        )
    if stale:
        problems.append(
            f"classified but no longer spawned anywhere: {', '.join(stale)}.\n"
            f"    Delete the entries - the lists are meant to track the tree, not outlive it."
        )

    if problems:
        print("spawned tools nobody has classified:\n")
        for p in problems:
            print(f"  - {p}")
        return 1

    print(
        f"{len(found)} external tool(s) spawned: {len(found) - len(ABSENT & found.keys())} on the image, "
        f"{len(ABSENT.keys() & found.keys())} absent with the surface each one costs. "
        f"Whether the absent ones SHOULD ship is a distribution decision this does not make."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
