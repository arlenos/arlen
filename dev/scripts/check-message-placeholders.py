#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A placeholder that reaches the reader as its own name.

The kit's translator is `messageformat` 4, which speaks MessageFormat 2: a
variable is `{$name}`, and `{name}` is a LITERAL expression - valid syntax that
formats to the word "name", wrapped in bidi isolates. So a message written the
older way does not fail, does not warn, and does not fall back. It renders
"Eject place" to the person holding the device, and passes every test in the
tree because nothing reads the rendered string.

WHY THIS IS A GATE. Five keys in the file manager were written that way - four
refusal sentences and an aria-label - and every one of them shipped the word
`place` or `scope` where the name of the thing belonged. They were found by
looking at a render, which is the only place they are visible: the catalogue
looks right, the call site passes the right value, and the two do not meet.

The rule: a message must not contain a bare `{identifier}`. A variable takes the
`$`, and a literal `{` in prose is escaped `{{`. That leaves the legitimate forms
- `{$name}`, `{$n :number}`, `{#markup}` - untouched.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else pathlib.Path(__file__).resolve().parents[2]
# The catalogues are `"key": "message",` lines; a message is the second string.
ENTRY = re.compile(r'^\s*"([^"]+)"\s*:\s*"((?:[^"\\]|\\.)*)"\s*,?\s*$')
BARE = re.compile(r"(?<!\{)\{\s*([A-Za-z_][A-Za-z0-9_]*)\s*\}")


# A BUILD OUTPUT IS NOT A SOURCE. `.svelte-kit/output` holds a compiled copy of
# every catalogue, so a first run reported all ten of the file manager's fixed
# messages as still broken - from a bundle written before the fix. A gate that
# reads generated files reports the past.
SKIP = {"node_modules", ".svelte-kit", "dist", "build", "target", ".vite"}


def catalogues(root: pathlib.Path) -> tuple[list[pathlib.Path], int]:
    """The catalogues to read, and how many the walk found before the skips.

    The second number separates the two ways of reading nothing. A tree whose
    only catalogue sits under a build directory is a real tree this deliberately
    does not read - its own control asserts that and expects the "0 catalogue(s)"
    line. A tree with no catalogue anywhere is a walk that reached nothing.
    """
    out = []
    found = 0
    for base in ("apps", "sdk"):
        d = root / base
        if not d.is_dir():
            continue
        for pattern in ("messages*.ts", "messages*.js"):
            hits = sorted(d.rglob(pattern))
            found += len(hits)
            out += [f for f in hits if not SKIP & set(f.parts)]
    return out, found


def findings(text: str) -> list[tuple[int, str, str]]:
    """Line, key and placeholder, for every bare `{identifier}` in a message."""
    out = []
    for i, line in enumerate(text.splitlines(), 1):
        m = ENTRY.match(line)
        if not m:
            continue
        for b in BARE.finditer(m.group(2)):
            out.append((i, m.group(1), b.group(1)))
    return out


def main() -> int:
    files, found = catalogues(ROOT)
    if found == 0:
        print("check-message-placeholders: no catalogue found, so the scan is pointed wrong")
        return 1
    hits = 0
    for f in files:
        for line, key, name in findings(f.read_text(encoding="utf-8", errors="replace")):
            hits += 1
            print(f"{f.relative_to(ROOT)}:{line}: `{key}` writes `{{{name}}}`")
            print(
                f"    MessageFormat 2 reads that as the literal word \"{name}\" and prints it. "
                f"Write `{{${name}}}` for the value, or `{{{{` for a brace in prose."
            )
    if hits:
        print(f"\n{hits} placeholder(s) that would print their own name")
        return 1
    print(
        f"{len(files)} catalogue(s) read: every placeholder names a variable, so none of "
        f"them reaches a reader as the word inside the braces."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
