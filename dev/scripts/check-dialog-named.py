#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A dialog with no accessible name.

A screen reader announces a dialog by its name. With none it says "dialog", and
the person is inside a modal that has taken their focus and told them nothing
about what it wants. The visible heading is right there on screen; the dialog
just has to point at it.

WHY THIS IS A GATE. Every hand-rolled `role="dialog"` in this tree was already
named - somebody had done that pass - and every bits-ui `Dialog.Content` had
been missed, because the primitive looks complete without a name and the kit
does not supply one. That is five modals in the shell, including the consent
card, which is the surface where a person grants or refuses authority: axe
reported it `serious` on all three request classes the first time that window
was rendered, on 6 September. A defect that hides behind a primitive is exactly
what a gate is for.

The rule: a dialog element must carry `aria-label` or `aria-labelledby`, or wrap
a `<Dialog.Title>` (the primitive's own naming path). A `<Dialog.Content>` is a
dialog; so is any tag carrying `role="dialog"` or `role="alertdialog"`.

The kit's own `components/ui/` wrappers are skipped: they are the primitive, and
the name belongs to whoever raises it.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else pathlib.Path(__file__).resolve().parents[2]
ROOTS = ["apps", "sdk"]
# The opening tag of a dialog, and everything up to its `>`. Attribute values may
# carry `>` inside a Svelte expression, so this counts braces rather than
# stopping at the first one: `role={x > 1 ? "dialog" : ""}` is not the shape here,
# but `aria-label={a > b ? p : q}` genuinely is.
OPEN = re.compile(r"<(Dialog\.Content|[A-Za-z][\w.-]*)\b")
NAMED = re.compile(r"\baria-label(ledby)?\s*=")
TITLE = re.compile(r"<Dialog\.Title\b")
ROLE = re.compile(r"\brole\s*=\s*[\"']?(?:\{[\"'])?(dialog|alertdialog)\b")


def tag_at(text: str, start: int) -> tuple[str, int]:
    """The opening tag beginning at `start`, and the index just past its `>`."""
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


def findings(text: str) -> list[tuple[int, str]]:
    """Line number and tag, for every dialog opened without a name."""
    out = []
    for m in OPEN.finditer(text):
        tag, end = tag_at(text, m.start())
        is_dialog = tag.startswith("<Dialog.Content") or ROLE.search(tag)
        if not is_dialog:
            continue
        if NAMED.search(tag):
            continue
        # The primitive's own naming path: a title inside the dialog's body. The
        # search is the rest of the file rather than the matching close tag,
        # because a `Dialog.Title` anywhere below a `Dialog.Content` is inside it
        # in every arrangement this tree uses, and being generous here costs
        # nothing a reviewer cannot see.
        if tag.startswith("<Dialog.Content") and TITLE.search(text, end):
            continue
        out.append((text.count("\n", 0, m.start()) + 1, " ".join(tag.split())[:120]))
    return out


def main() -> int:
    files = []
    for r in ROOTS:
        base = ROOT / r
        if base.is_dir():
            files += sorted(base.rglob("*.svelte"))
    files = [
        f
        for f in files
        if "node_modules" not in f.parts and "ui" not in f.relative_to(ROOT).parts[-3:-1]
    ]
    hits = 0
    for f in files:
        for line, tag in findings(f.read_text(encoding="utf-8", errors="replace")):
            hits += 1
            print(f"{f.relative_to(ROOT)}:{line}: this dialog has no accessible name")
            print(f"    {tag}")
            print(
                "    A screen reader announces it as \"dialog\". Give it `aria-labelledby` "
                "pointing at its own heading, or `aria-label`."
            )
    if hits:
        print(f"\n{hits} dialog(s) with no accessible name")
        return 1
    print(
        f"{len(files)} component(s) read: every dialog names itself, so none is announced "
        f"as an unnamed modal that has taken the focus."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
