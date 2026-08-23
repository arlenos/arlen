#!/usr/bin/env python3
"""A topbar menu label must not be a Rust prose literal.

The shell renders `os_sdk::menu` labels verbatim and holds no catalog to
translate them against, so a label written in Rust is the source language
forever. Files did exactly that: the whole bar read "File Edit View Go Help"
over a window whose own chrome was German, and because the menu was published
from `setup()` - before the webview that knows the language exists - a later
language switch could not reach it either. The tree now comes from the
frontend, where the catalog is.

What this refuses: a string literal in the label position of
`MenuItem::item`, `MenuItem::submenu` or `MenuGroup::new` that reads as prose.
An id-shaped literal ("view.sort.name") is fine; those are action ids, not
labels. A label built from a variable is fine - that is the whole point.

Scope: every Rust file in the tree except the surface's own crate, whose
tests and doc examples must be able to name a literal label to test with.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SKIP_DIRS = {"target", "node_modules", ".git"}
# The surface itself: its tests construct menus, and a test fixture is not a
# shipped label.
SKIP_PREFIXES = ("sdk/os-sdk/src/menu.rs",)

CALLS = re.compile(
    r"""\b(?:MenuItem::item|MenuItem::submenu|MenuGroup::new)\s*\(\s*"([^"]*)\"""",
    re.VERBOSE,
)

# An action id, not a label: dotted or snake, no spaces, all lowercase.
ID_SHAPED = re.compile(r"^[a-z0-9_]+(?:[._][a-z0-9_]+)*$")


def prose(literal: str) -> bool:
    if not literal.strip():
        return False
    if ID_SHAPED.match(literal):
        return False
    return any(c.isalpha() for c in literal)


def rust_files(root: Path):
    for p in root.rglob("*.rs"):
        rel = p.relative_to(root).as_posix()
        if any(part in SKIP_DIRS for part in p.parts):
            continue
        if rel.startswith(SKIP_PREFIXES):
            continue
        yield p, rel


def main() -> int:
    findings = []
    checked = 0
    for path, rel in rust_files(ROOT):
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        hits = list(CALLS.finditer(text))
        if not hits:
            continue
        checked += 1
        for m in hits:
            literal = m.group(1)
            if not prose(literal):
                continue
            line = text.count("\n", 0, m.start()) + 1
            findings.append((rel, line, literal))

    for rel, line, literal in findings:
        print(f"{rel}:{line}: menu label written in Rust: {literal!r}")
    if findings:
        print()
        print(
            "A menu label is rendered to a reader as it arrives. Publish the tree\n"
            "from the frontend, where the catalog is, and re-publish on a language\n"
            "change - see apps/files/src/lib/menu.ts."
        )
        return 1
    print(
        f"check-menu-labels-translated: {checked} file(s) name a menu label in Rust, "
        "none of them prose."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
