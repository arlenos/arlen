#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""An id that does not identify one element.

`<label for="x">` and `aria-labelledby="x"` are references, and a reference that
resolves to no element does exactly nothing: the label does not name the control,
clicking it moves no focus, and the markup reads correct to anybody skimming it.
Nothing throws, nothing warns, and axe only catches part of it.

WHY THIS IS A GATE. Two of them were in the tree, and both had the same cause: a
`<label for="...">` written beside a kit component that takes an `ariaLabel` and
forwards no `id` - the screenshot app's capture-source picker and the settings
window-rule dialog's dialog-match switch. The control is named, so a reader sees
a label and a name and moves on; the `for` is the part that quietly means nothing.

The rule: an id named by `for`, `aria-labelledby`, `aria-describedby`,
`aria-controls` or `aria-owns` must exist in the same file - either as an
`id="..."` or as an attribute value handed to a component, which is how the
shell's dialogs pass a title id down (`titleId="sh-consent-title"`). So a name
that appears only ONCE in its file is a reference to nothing; a name that appears
twice has somewhere to land, and where it lands is a reviewer's question rather
than this check's.

Dynamic references (`aria-labelledby={expr}`) are not read: the id is computed and
this cannot follow it.

THE OTHER HALF IS A TARGET THAT WILL EXIST TWICE. A literal `id="x"` inside an
`{#each}` renders once per row, so a page with two fixable posture lines has two
elements claiming the same id - invalid, and it makes every reference to it
ambiguous. No per-file count can see that; it needs the block. The physical
privacy page had one, on a Fix button nothing referenced, which is the shape
that survives: an id nobody uses is an id nobody notices repeating.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else pathlib.Path(__file__).resolve().parents[2]
ROOTS = ["apps", "sdk"]
REF = re.compile(r'\b(aria-labelledby|aria-describedby|aria-controls|aria-owns|for)="([^"{}]+)"')
#: COMMENTS ARE NOT MARKUP, and in this tree that matters more than usual: the
#: habit here is to explain a fix by quoting the attribute it removed, so the
#: first run after fixing the screenshot app's label reported the same finding
#: from inside the sentence describing the fix. Both comment forms go, and they
#: go before anything is counted, so a quoted id cannot stand in for a target
#: either.
COMMENT = re.compile(r"<!--.*?-->|/\*.*?\*/", re.S)
IDS = re.compile(r'\bid="([^"{}]+)"')


def findings(text: str) -> list[tuple[int, str, str]]:
    """Line, kind and id, for every reference that names nothing."""
    # Blanked rather than removed, so the line numbers still point at the file.
    text = COMMENT.sub(lambda m: re.sub(r"[^\n]", " ", m.group(0)), text)
    out = []
    for m in REF.finditer(text):
        for token in m.group(2).split():
            # The reference itself is one occurrence; a target is a second.
            if text.count(f'"{token}"') > 1:
                continue
            out.append((text.count("\n", 0, m.start()) + 1, m.group(1), token))
    return out


def repeated(text: str) -> list[tuple[int, str]]:
    """Line and id, for every literal id written inside an `{#each}` block."""
    text = COMMENT.sub(lambda m: re.sub(r"[^\n]", " ", m.group(0)), text)
    out = []
    depth = 0
    for number, line in enumerate(text.splitlines(), 1):
        if depth > 0:
            for m in IDS.finditer(line):
                out.append((number, m.group(1)))
        depth += line.count("{#each") - line.count("{/each}")
    return out


def main() -> int:
    files = []
    for r in ROOTS:
        base = ROOT / r
        if base.is_dir():
            files += sorted(base.rglob("*.svelte"))
    files = [f for f in files if "node_modules" not in f.parts]
    if not files:
        print("check-id-references: no Svelte source found, so the scan is pointed wrong")
        return 1
    hits = 0
    for f in files:
        text = f.read_text(encoding="utf-8", errors="replace")
        for line, attr, token in findings(text):
            hits += 1
            print(f"{f.relative_to(ROOT)}:{line}: {attr}=\"{token}\" names nothing")
            print(
                "    No element in this file carries that id and nothing hands it to one, "
                "so the reference resolves to no element at all. Give the target the id, or "
                "drop the reference and let the control's own name stand."
            )
        for line, token in repeated(text):
            hits += 1
            print(f"{f.relative_to(ROOT)}:{line}: id=\"{token}\" is written inside an each block")
            print(
                "    It renders once per row, so every row past the first repeats it and no "
                "reference can say which one it means. Build it from the row's own key, or "
                "drop it if nothing refers to it."
            )
    if hits:
        print(f"\n{hits} id(s) that do not identify one element")
        return 1
    print(
        f"{len(files)} component(s) read: every `for` and every aria reference names "
        f"something in its own file, and no literal id is written where it would repeat."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
