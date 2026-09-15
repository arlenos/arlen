#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A source file `grep` reads as binary, which is a file nothing can search.

A NUL byte anywhere in a file makes `file` call it `data` and makes `grep` refuse
it: no output, exit 1, no message. That is indistinguishable from "no match", so
every search over that file quietly comes back empty and every reader believes it.

FOUND BY ACCIDENT, which is the argument for the gate. `sdk/ui-kit/src/lib/i18n/
index.ts` builds a translator cache key as `` `${loc}<NUL>${id}` `` - a sound
separator, since a NUL appears in neither a locale tag nor a message id - but it
was written as the literal byte instead of the `\\u0000` escape. The string is
identical either way; the FILE is not. For as long as that byte was there, the
whole i18n runtime was invisible to grep, and on 16 September that produced a
confident wrong reading ("`dir` is imported from the kit and the kit does not
export it") that took reading the raw bytes to undo. The escape fixes it with no
change to what runs.

The rule is therefore not "no NUL in a string" - it is "no NUL in a source FILE".
Write the escape.
"""

import pathlib
import sys

ROOT = pathlib.Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else pathlib.Path(__file__).resolve().parents[2]
# Text formats only. A `.png` is binary on purpose and a `.raw` is an image.
SUFFIXES = {
    ".ts", ".tsx", ".js", ".mjs", ".cjs", ".svelte", ".rs", ".py", ".sh", ".bash",
    ".toml", ".json", ".css", ".html", ".yml", ".yaml", ".proto", ".md", ".service",
    ".desktop", ".xml", ".sql",
}
# `terminfo` earns its place here: a compiled terminfo database names each entry
# after the TERMINAL, not after a file format, so the image ships one called
# `xterm.js` which is binary by definition and not JavaScript at all. Excluding it
# by path rather than widening the rule, because the rule is right.
SKIP_DIRS = {"node_modules", "target", ".git", ".svelte-kit", "dist", "build", "terminfo"}


def scanned(root: pathlib.Path) -> list[pathlib.Path]:
    """Every text source file under `root`."""
    return [
        f
        for f in sorted(root.rglob("*"))
        if f.is_file() and f.suffix in SUFFIXES and not (SKIP_DIRS & set(f.parts))
    ]


def offenders(files: list[pathlib.Path]) -> list[tuple[pathlib.Path, int, int]]:
    """Path, NUL count, and the line the first one is on."""
    out = []
    for f in files:
        try:
            b = f.read_bytes()
        except OSError:
            continue
        if b"\x00" in b:
            out.append((f, b.count(b"\x00"), b[: b.index(b"\x00")].count(b"\n") + 1))
    return out


def main() -> int:
    files = scanned(ROOT)
    if not files:
        print("check-source-is-text: no text source found, so the scan is pointed wrong")
        return 1
    hits = offenders(files)
    for f, count, line in hits:
        print(f"{f.relative_to(ROOT)}:{line}: {count} NUL byte(s) in a text source file")
        print(
            "    `grep` reads this file as binary and answers nothing, exit 1, which reads "
            "exactly like no match. If the NUL is a deliberate separator, write it as the "
            "escape `\\u0000` (JS/TS) or `\\0` (Rust): same string, text file."
        )
    if hits:
        print(f"\n{len(hits)} source file(s) that grep cannot search")
        return 1
    print(
        f"{len(files)} text source file(s) are text: no NUL byte, so grep answers about all "
        f"of them rather than silently skipping one."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
