#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""An ellipsis a flex or grid box cannot honour.

`text-overflow` applies to a block container whose INLINE content overflows. A
flex or grid container has items, not inline content, so the declaration does
nothing there - and what a person sees is the text cut mid-glyph, with no
ellipsis and no sign that anything is missing.

WHY THIS IS A GATE. It is not a style opinion; it is a rule that reads as done
and is inert. Found twice on 6 September by rendering, in two components neither
of which looked wrong in the source: the knowledge timeline drew "Quarterly
report.pd" - a filename that lost its extension and still looked like a filename
- and the saved-search list cut a project chip in half beside it. Both carried
`display: inline-flex` with `overflow: hidden; text-overflow: ellipsis;
white-space: nowrap` in the same rule, which is a complete and correct-looking
ellipsis recipe that cannot run.

THE FIX IS ALWAYS THE SAME SHAPE: move the words into a child that IS a text box
and let the flex container keep the layout. Then the child ellipses and the
siblings beside it keep their size.

Scans `<style>` blocks in `.svelte` files, rule by rule. A rule that sets both is
the finding; a file that happens to contain both in different rules is not, which
is why this parses rather than greps.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else pathlib.Path(__file__).resolve().parents[2]
ROOTS = ["apps", "sdk"]
STYLE = re.compile(r"<style[^>]*>(.*?)</style>", re.S)
# A rule is `selector { declarations }` with no nested braces. At-rules with
# blocks (`@media`) are unwrapped first so their inner rules are seen.
RULE = re.compile(r"([^{}]+)\{([^{}]*)\}", re.S)
FLEXY = re.compile(r"display\s*:\s*(inline-)?(flex|grid)\b")
ELLIPSIS = re.compile(r"text-overflow\s*:\s*ellipsis\b")


def findings(text: str) -> list[tuple[str, str]]:
    """Selector and its declarations, for every rule that sets both."""
    out = []
    for block in STYLE.findall(text):
        for selector, decls in RULE.findall(block):
            if FLEXY.search(decls) and ELLIPSIS.search(decls):
                out.append((" ".join(selector.split()), " ".join(decls.split())))
    return out


def main() -> int:
    files = []
    for r in ROOTS:
        base = ROOT / r
        if base.is_dir():
            files += sorted(base.rglob("*.svelte"))
    files = [f for f in files if "node_modules" not in f.parts]
    hits = 0
    for f in files:
        for selector, decls in findings(f.read_text(encoding="utf-8", errors="replace")):
            hits += 1
            rel = f.relative_to(ROOT)
            print(f"{rel}: `{selector}` declares an ellipsis it cannot honour")
            print(f"    {decls[:160]}")
            print(
                "    `text-overflow` needs a box with inline content; this one has flex or "
                "grid items. Move the words into a child that is a text box."
            )
    if hits:
        print(f"\n{hits} rule(s) declaring an ellipsis on a flex or grid box")
        return 1
    print(
        f"{len(files)} component(s) checked: no rule declares `text-overflow: ellipsis` on a "
        f"flex or grid box, where it does nothing and the words are cut mid-glyph instead."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
