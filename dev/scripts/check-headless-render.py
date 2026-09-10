#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Check that nothing renders on the developer's own screen.

WHY. `render-wide.py` drives a real browser window. It inherits whatever `DISPLAY`
and `WAYLAND_DISPLAY` its caller has, so a script that invokes it without an Xvfb
around it opens that window on the screen of whoever ran the script - interrupting
them, and measuring the page under their window manager rather than the fixed
off-screen one everything in `dev/screenshot` is meant to render against.

This is not hypothetical and it is not old. `headless.sh` was written in August
after a run drew on the developer's session, and its header says so. On 7 September
`sweep-axe.sh` was found calling `render-wide.py` directly, twice - so an
accessibility sweep of fifteen apps opened fifteen real windows, one per app, on
whoever ran it. It had done that since it was written, and nothing said a word,
because the rule lived in a comment in a different file. Half an hour later I did
the same thing by hand while chasing that bug, which is the argument for a check
rather than a sentence: the rule is easy to know and easy to forget.

THE RULE. A file that invokes `render-wide.py` must either be `headless.sh` - the
one place that owns the recipe - or set up its own `xvfb-run`. Two files
legitimately do the second (`shoot-no-backend.sh`, which needs a production build
its own way, and `test-render-wide.sh`, which is the renderer's own control), and
both carry the full recipe including a window manager.

WHAT IT DOES NOT CATCH, said plainly: the rule is per FILE, so a script that wraps
one call in `xvfb-run` and leaves another bare passes. Scoping it to the enclosing
block means parsing shell, which is a great deal of machinery for a class whose
whole population is five files. If that shape ever appears, tighten it then.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = (
    Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
)

RENDERER = "render-wide.py"
# The file that owns the recipe, and so the one file allowed to name the renderer
# with no Xvfb of its own in sight - it IS the Xvfb.
OWNER = "headless.sh"


def code_lines(text: str) -> list[str]:
    """The lines that could be a command: comments and blank lines dropped.

    Deliberately crude, and it can only be crude in the SAFE direction: a
    commented mention is skipped, so the check never fires on a sentence about
    the renderer - `shoot-no-backend.sh` has one telling you to use `headless.sh`
    and it must not be a finding.
    """
    out = []
    for line in text.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        out.append(line)
    return out


def offenders(root: Path) -> tuple[list[tuple[str, int]], int, int]:
    """The bare calls, plus how many files were read and how many name the renderer.

    The counts are returned because the summary prints them: a run that examined
    nothing and a run that examined forty both said "ok" until this did, and this
    tree has already been bitten by a check reporting a clean tally over an empty
    set.
    """
    found: list[tuple[str, int]] = []
    read = 0
    naming = 0
    for path in sorted(root.rglob("*")):
        if path.name == RENDERER or not path.is_file():
            continue
        if path.suffix not in (".sh", ".py"):
            continue
        if "node_modules" in path.parts or "target" in path.parts:
            continue
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        read += 1
        if RENDERER not in text:
            continue
        naming += 1
        lines = code_lines(text)
        calls = [ln for ln in lines if RENDERER in ln]
        if not calls:
            continue
        if path.name == OWNER:
            continue
        # `xvfb-run` anywhere in the file's code is the recipe being set up here -
        # and so is `own_display`, which IS that recipe since the display-number
        # loop moved into `lib/own-display.sh`. Both spellings, because the
        # scripts that build their own Xvfb went through the helper on
        # 11 September and this check reported every one of them as rendering
        # bare: a gate matches the shape its author last happened to write.
        if any("xvfb-run" in ln or "own_display" in ln for ln in lines):
            continue
        for i, line in enumerate(text.splitlines(), 1):
            if RENDERER in line and line.strip() and not line.strip().startswith("#"):
                found.append((str(path.relative_to(root)), i))
    return found, read, naming


def main() -> int:
    bad, read, naming = offenders(ROOT / "dev")
    if read == 0:
        print("check-headless-render: no script under dev/ was read, so the scan is pointed wrong")
        return 1
    if not bad:
        print(
            f"{read} script(s) under dev/ read, {naming} of them invoke the renderer, "
            "each through headless.sh or its own Xvfb"
        )
        return 0
    print("A render runs on whoever's screen is attached:")
    for rel, line in bad:
        print(f"  {rel}:{line} calls {RENDERER} with no Xvfb around it")
    print()
    print(f"  Call dev/screenshot/{OWNER} instead - it supplies the Xvfb at a real")
    print("  size, cuts off the host session, and starts a window manager so the")
    print("  page can go fullscreen. Every argument is passed straight through.")
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
