#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A control inside a list option, which is a listbox row nobody can reach.

The kit's `CommandItem` renders as `role="option"` inside a `role="listbox"`, and
an option may not contain an interactive element. ARIA says so, axe reports it as
`nested-interactive`, and the practical half is worse than the standards half: a
composite widget keeps focus on its own input and moves a SELECTION with the arrow
keys, so Tab never lands inside a row. A button in there is reachable with a mouse
and by nothing else.

WHY THIS IS A GATE AND NOT A SWEEP FINDING. axe finds it, and axe only looks at
surfaces a sweep actually renders. Three instances existed in this tree on
16 September - the terminal's quick-connect palette, the shell's clipboard popover,
and the launcher's clipboard group - and only ONE of them was on a swept surface.
The clipboard popover is in no sweep table; the launcher's group needs an opt-in
flag and a plugin bridge before it draws at all. So two of the three were invisible
to the tool that is supposed to find them, for as long as they existed. This check
reads the source, so a row nobody has photographed is a row it can still judge.

THE FIX IS THE SAME SHAPE EVERY TIME: the action leaves the row. The kit's
`Command` root reports its highlighted item through a bindable `value`, so a
footer control can act on whatever the selection is on - outside the listbox,
Tab-reachable, and it names the row it acts on. The other sound answer is the grid
pattern (`grid`/`row`/`gridcell`), which is a different widget and a bigger change.

WHAT IT CANNOT SEE: a snippet DEFINED elsewhere and passed into an item. The three
instances all wrote the control inline (one inside an inline `{#snippet}`), which
is the shape a person reaches for, but a gate that claimed to cover the other one
would be lying.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else pathlib.Path(__file__).resolve().parents[2]
ROOTS = ["apps", "sdk"]

# The components that render as a single ARIA option or menu item. A `role` this
# check trusts because the kit sets it, not because the markup says so.
OPTION = re.compile(r"<(CommandItem|SelectItem|ComboboxItem|DropdownMenu\.Item|Select\.Item)\b")
# What may not be inside one. `<a>` without an href is not a link and not
# interactive, so it is not listed; a bare `role="button"` on any element is.
CONTROL = re.compile(
    r"<(button|input|select|textarea)\b"
    r'|<a\b[^>]*\bhref='
    r'|role="(button|link|checkbox|switch|menuitem|option|tab)"'
)


def findings(text: str) -> list[tuple[int, str, str]]:
    """Line, the option component, and the offending line, for each nesting."""
    out: list[tuple[int, str, str]] = []
    lines = text.split("\n")
    depth = 0
    tag = ""
    for n, line in enumerate(lines, 1):
        if depth == 0:
            m = OPTION.search(line)
            if not m:
                continue
            tag = m.group(1)
            after = line[m.end():]
            # `<X ... />` holds nothing at all.
            if line.rstrip().endswith("/>"):
                continue
            # `<X ...>…</X>` on one line holds whatever is BETWEEN them, and only
            # that: a control after the close is a sibling, not a child. Skipping
            # the line outright was the first cut and it missed a one-line item
            # with a link in it, which is how the control found this.
            close = after.find(f"</{tag}>")
            if close != -1:
                inner = after[:close]
                if CONTROL.search(inner):
                    out.append((n, tag, inner.strip() or line.strip()))
                continue
            depth = 1
            continue
        if CONTROL.search(line):
            out.append((n, tag, line.strip()))
        if f"</{tag}>" in line:
            depth = 0
    return out


def main() -> int:
    files: list[pathlib.Path] = []
    for r in ROOTS:
        base = ROOT / r
        if base.is_dir():
            files += sorted(base.rglob("*.svelte"))
    files = [f for f in files if "node_modules" not in f.parts]
    if not files:
        print("check-nested-interactive: no Svelte source found, so the scan is pointed wrong")
        return 1
    hits = 0
    for f in files:
        for line, tag, text in findings(f.read_text(encoding="utf-8", errors="replace")):
            hits += 1
            print(f"{f.relative_to(ROOT)}:{line}: a control inside a <{tag}>")
            print(f"    {text[:120]}")
            print(
                "    An option may not contain a control, and the keyboard cannot reach one: "
                "focus stays in the palette's input. Move the action beside the list, on the "
                "row the selection is on (`Command` reports it through a bindable `value`)."
            )
    if hits:
        print(f"\n{hits} control(s) inside a list option")
        return 1
    print(
        f"{len(files)} component(s) checked: no list option contains a control. A row's action "
        f"lives beside the list, where the keyboard can reach it."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
