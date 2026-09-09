#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A handler declared before `{...props}` is a handler that never runs.

A bits-ui trigger hands its own attributes to a `child` snippet, and its props
include event handlers - `onclick`, `onpointerdown`, `onfocus`, `onblur`. Svelte
applies attributes in source order, so a later spread REPLACES an earlier
attribute of the same name. Write the handler first and the spread second, and
the component's handler silently wins: the button looks wired, reads wired, and
does nothing.

WHY THIS IS A GATE. Three buttons in the shell were dead this way and nobody had
noticed, because each is inside a tooltip whose own handler still fires - so the
control reacts, just not with its own behaviour. The top bar's now-playing art
was the worst of them: clicking it was the ONLY way to open the media panel, so
that whole panel had been unreachable for as long as the applet has existed, and
it took a render with a host script to find out. The quick-settings mute button
and the player switcher were the same shape.

The rule, inside a `{#snippet child({ props })}`: `{...props}` comes before any
`on*` attribute on that element. Put the spread first and call the component's
handler from yours (`props.onclick?.(e)`) so both still run.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else pathlib.Path(__file__).resolve().parents[2]
ROOTS = ["apps", "sdk"]
CHILD = re.compile(r"\{#snippet\s+child\s*\(\s*\{\s*props\s*\}")
# An element opening inside that snippet. Only the tag matters, so the scan is
# per opening tag rather than per line: a handler and the spread routinely sit on
# different lines of the same tag.
OPEN = re.compile(r"<[A-Za-z][\w.-]*\b")
SPREAD = re.compile(r"\{\s*\.\.\.\s*props\s*\}")
HANDLER = re.compile(r"(?<![\w.$])(on[a-z]+)\s*=")


def tag_at(text: str, start: int) -> tuple[str, int]:
    """The opening tag beginning at `start`, and the index just past its `>`.

    Brace-aware, because an attribute value is a Svelte expression and may hold a
    `>` of its own: `onclick={() => go()}` stops a naive scan at the arrow.
    """
    depth = 0
    i = start
    while i < len(text):
        c = text[i]
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
        elif c == ">" and depth == 0:
            return text[start : i + 1], i + 1
        i += 1
    return text[start:], len(text)


def findings(text: str) -> list[tuple[int, str, str]]:
    """Line, handler name and tag, for every handler shadowed by a later spread."""
    out = []
    for c in CHILD.finditer(text):
        # The snippet body, bounded by its `{/snippet}`. A file may hold several.
        end = text.find("{/snippet}", c.end())
        body_end = end if end != -1 else len(text)
        i = c.end()
        while i < body_end:
            m = OPEN.search(text, i, body_end)
            if not m:
                break
            tag, after = tag_at(text, m.start())
            spread = SPREAD.search(tag)
            if spread:
                for h in HANDLER.finditer(tag):
                    if h.start() < spread.start():
                        line = text.count("\n", 0, m.start() + h.start()) + 1
                        out.append((line, h.group(1), " ".join(tag.split())[:120]))
                        break
            i = max(after, m.end())
    return out


def main() -> int:
    files = []
    for r in ROOTS:
        base = ROOT / r
        if base.is_dir():
            files += sorted(base.rglob("*.svelte"))
    files = [f for f in files if "node_modules" not in f.parts]
    if not files:
        print("check-child-props-order: no Svelte source found, so the scan is pointed wrong")
        return 1
    hits = 0
    for f in files:
        for line, handler, tag in findings(f.read_text(encoding="utf-8", errors="replace")):
            hits += 1
            print(f"{f.relative_to(ROOT)}:{line}: `{handler}` is declared before `{{...props}}`")
            print(f"    {tag}")
            print(
                f"    The spread replaces it, so this control runs the component's `{handler}` "
                f"and never its own. Put `{{...props}}` first and call "
                f"`props.{handler}?.(e)` from yours."
            )
    if hits:
        print(f"\n{hits} handler(s) replaced by a later props spread")
        return 1
    print(
        f"{len(files)} component(s) read: every handler inside a `child` snippet is declared "
        f"after the spread, so none of them is silently replaced."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
