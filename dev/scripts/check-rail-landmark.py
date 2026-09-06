#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A navigation rail that is in no landmark.

The kit's `Sidebar` is divs down to the primitive - it takes no role and renders
no `<nav>` - so an app that composes it gets a rail that a screen reader cannot
jump to, cannot skip past, and does not announce as navigation. The content is
all there and none of it is reachable as a group.

WHY THIS IS A GATE. EIGHT of the thirteen apps had it at once, which is what a
missing floor looks like: settings, files, knowledge, terminal, calendar, pdf,
meetings, mail. It is not a thing anybody forgot once. And it survived every
sweep the tree had, because axe's own `region` rule stays quiet about a rail
whose text all sits inside buttons - mail read clean and was not.

The rule: a component that renders `<Sidebar` must also carry a landmark - a
`<nav>`, or `role="navigation"`, or `role="search"` for a rail that is only a
search box. Where the header and footer sit outside `SidebarContent`, the
landmark goes around the whole rail; the terminal's does.

`sdk/ui-kit/src/lib/components/ui/` is skipped: that is the primitive itself,
and whether IT should carry the role is the kit's decision, not this check's.
`apps/harness` and `apps/store` are skipped for the reason the sweep tables give
for leaving them out: they are arlen-ui's live work, and a shared gate that goes
red on another lane's surface is a gate somebody turns off. Both of their rails
have this defect today, and it is theirs to close.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else pathlib.Path(__file__).resolve().parents[2]
ROOTS = ["apps", "sdk"]
SIDEBAR = re.compile(r"<Sidebar(?![A-Za-z])")
LANDMARK = re.compile(r"<nav[\s>]|role\s*=\s*[\"'{]?[\"']?(navigation|search)\b")


def main() -> int:
    files = []
    for r in ROOTS:
        base = ROOT / r
        if base.is_dir():
            files += sorted(base.rglob("*.svelte"))
    other_lane = {("apps", "harness"), ("apps", "store")}
    files = [
        f
        for f in files
        if "node_modules" not in f.parts
        and "ui" not in f.relative_to(ROOT).parts[-3:-1]
        and f.relative_to(ROOT).parts[:2] not in other_lane
    ]
    rails = [f for f in files if SIDEBAR.search(f.read_text(encoding="utf-8", errors="replace"))]
    hits = 0
    for f in rails:
        text = f.read_text(encoding="utf-8", errors="replace")
        if LANDMARK.search(text):
            continue
        hits += 1
        line = text.count("\n", 0, SIDEBAR.search(text).start()) + 1
        print(f"{f.relative_to(ROOT)}:{line}: this rail is in no landmark")
        print(
            "    The kit's Sidebar renders divs, so everything in it is content a reader "
            "cannot jump to. Wrap it in a named `<nav>`, or mark a search-only rail "
            "`role=\"search\"`."
        )
    if hits:
        print(f"\n{hits} rail(s) with no landmark")
        return 1
    print(
        f"{len(rails)} rail(s) read across {len(files)} component(s): each one announces "
        f"itself as navigation rather than as loose content."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
