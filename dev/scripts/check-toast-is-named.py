#!/usr/bin/env python3
"""A toast the shell raises is NAMED here and WRITTEN in the frontend.

The shell's backend has no catalog: the message catalog is in the webview. So a
sentence built on this side is a sentence in the source language, and that is
what a German desktop showed - twenty-one quick actions answering "Night Light
is now on" after the switch flipped, the assistant that "did not open", the
command that "did not run", the app whose launcher entry was "malformed".

`emit_toast_key(app, kind, id, params)` names the line and the frontend writes
it. `emit_toast` still exists for a line that is already in the reader's
language, but a PROSE LITERAL handed to it is the defect this refuses: the
backend cannot know the words.

What counts as prose: a literal with a space in it and a letter, in the call's
argument list, including inside a `format!`. A catalog id (`sh.toast.x`) is not
prose; neither is a short token with no space.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
# `mkosi.builddir` and its siblings hold a cargo cache with git CHECKOUTS of
# older copies of this very tree, so scanning them reports defects that were
# fixed months ago in files nobody edits. Skipped by every gate that walks the
# whole repo, and this one learned it the hard way: its first run named a line
# in a cached checkout of an arlen commit from June.
SKIP_DIRS = {
    "target",
    "node_modules",
    ".git",
    "mkosi.builddir",
    "mkosi.cache",
    "mkosi.tools",
}
CALL = "emit_toast("


def call_bodies(text: str):
    """Each `emit_toast(...)` argument list, with the line it starts on."""
    for m in re.finditer(r"(?<![a-z_])emit_toast\s*\(", text):
        # `emit_toast_key(` shares the prefix up to the paren; it is named by
        # construction and is the thing this gate steers callers towards.
        if re.search(r"emit_toast_key\s*\($", text[: m.end()]):
            continue
        depth, i = 0, m.end() - 1
        while i < len(text):
            if text[i] == "(":
                depth += 1
            elif text[i] == ")":
                depth -= 1
                if depth == 0:
                    break
            i += 1
        yield text.count("\n", 0, m.start()) + 1, text[m.end() : i]


PROSE = re.compile(r'"([^"\\]*(?:\\.[^"\\]*)*)"')


def prose_in(body: str):
    for lit in PROSE.findall(body):
        # `{id}: {e}` is punctuation around two placeholders, not a sentence.
        # Judging it as prose made the first run report a format string that
        # says nothing in any language.
        stripped = re.sub(r"\{[^}]*\}", "", lit).strip()
        if " " not in stripped:
            continue
        if not any(c.isalpha() for c in stripped):
            continue
        yield lit


def main() -> int:
    findings = []
    checked = 0
    for path in sorted(ROOT.rglob("*.rs")):
        if any(part in SKIP_DIRS for part in path.parts):
            continue
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        if CALL not in text:
            continue
        rel = path.relative_to(ROOT).as_posix()
        for line, body in call_bodies(text):
            checked += 1
            for lit in prose_in(body):
                findings.append(f"{rel}:{line}: toast written in Rust: {lit!r}")

    for f in findings:
        print(f"  - {f}")
    if findings:
        print()
        print(
            "The catalog is in the frontend. Name the line with\n"
            "`emit_toast_key(app, kind, \"sh.toast.something\", &[(\"why\", cause)])`\n"
            "and add the sentence to `messages.ts` in every locale."
        )
        return 1
    print(f"check-toast-is-named: {checked} toast call(s), none of them written here.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
