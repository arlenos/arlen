#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a height or radius fallback anywhere is its token.

A component writes `height: var(--height-control, 30px)`, and the fallback is
what renders where the token is not defined. Both halves are a statement about
the same number, written in two files, and on 9 September all one hundred
and nineteen of them disagreed: every fallback was exactly 2px under its token, which is what happens
when the height register is raised once and the components are not.

WHY IT MATTERS EVEN WHERE THE TOKEN IS ALWAYS THERE. Today every app and the
picker UI define the tokens, so the fallbacks are unreachable and nothing renders
wrong. What they do instead is teach the wrong number: `sizing-system.md` says the
height register IS the design, and a reader who opens a component to find out how
tall a control is reads the fallback, because it is the literal on the line. Forty
stale literals are forty chances to build the next control 2px short.

WHAT IT DOES NOT CHECK. Whether a fallback should exist at all - a component that
renders outside the kit's stylesheet needs one, and one that never does is free to
drop it. This only says the two numbers agree.

Run: dev/scripts/check-size-fallbacks.py [root]
"""

import re
import sys
from pathlib import Path

#: The named registers this checks: the height ladder and the radius ladder.
#: Both are a fixed set of numbers the design decides once, which is what makes a
#: disagreeing fallback a drift rather than a local choice. The generic
#: `--radius-sm/md/lg/xl` aliases are deliberately NOT in it - they are the
#: shadcn-derived back-compat names and a component using one is already saying
#: it does not mean the Arlen register.
TOKEN = re.compile(r"^\s*(--(?:height|radius)-(?:chip|button|input|card|modal|[a-z-]*)):\s*(\d+)px;", re.M)
USE = re.compile(r"var\((--(?:height|radius)-[a-z-]+),\s*(\d+)px\)")


def main() -> int:
    root = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
    css = root / "sdk/ui-kit/src/app.css"
    trees = [root / "sdk/ui-kit/src/lib", root / "apps", root / "daemons"]
    if not css.is_file() or not trees[0].is_dir():
        print("check-size-fallbacks: no kit stylesheet or component tree, so the scan is pointed wrong")
        return 1

    tokens = {m.group(1): int(m.group(2)) for m in TOKEN.finditer(css.read_text(errors="replace"))}
    if not tokens:
        print("check-size-fallbacks: the kit stylesheet declares no height tokens, which cannot be right")
        return 1

    checked = 0
    problems: list[str] = []
    paths = [
        path
        for tree in trees
        if tree.is_dir()
        for path in sorted(tree.rglob("*.svelte"))
        if not any(part in str(path) for part in ("node_modules", "/build/", "/.svelte-kit/"))
    ]
    for path in paths:
        for m in USE.finditer(path.read_text(errors="replace")):
            name, fallback = m.group(1), int(m.group(2))
            if name not in tokens:
                continue
            checked += 1
            if tokens[name] != fallback:
                problems.append(
                    f"  - {path.relative_to(root)}: {name} falls back to {fallback}px, "
                    f"and the token is {tokens[name]}px"
                )

    if problems:
        print("Height fallbacks that disagree with their token:\n")
        print("\n".join(problems))
        print(
            "\n  The fallback is what renders where the token is missing, and it is"
            "\n  what a reader takes for the intended height. Move it to the token."
        )
        return 1

    print(f"check-size-fallbacks: {checked} height and radius fallback(s) across the kit, the apps "
        "and the daemons; each is its token")
    return 0


if __name__ == "__main__":
    sys.exit(main())
