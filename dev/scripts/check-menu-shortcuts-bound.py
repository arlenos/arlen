#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Check that a keystroke a menu advertises is one the app has learnt.

THE SHAPE. A menu item may carry an accelerator beside its label - `Save
Ctrl+S`, `Compose Ctrl+N` - and that text is a promise about the keyboard. The
shell renders it verbatim from the registered menu; nothing anywhere checks that
the app listens for it. So an app can advertise a keystroke it has never bound
and the only way to find out is to press it.

Found on 9 September by reading the eleven menus after the relay landed. The
relay made every menu ITEM work; the accelerators beside them were a separate
claim nobody had measured. One was false: mail's Message menu said `Ctrl+N` over
a window with no keydown handler at all, so the menu was the only thing saying
the key existed.

THE SECOND ONE WAS THIS CHECK BEING WRONG, and it is the reason the matcher
below is shaped the way it is. The first cut called the text editor's `Ctrl+S`
unbound too. It is bound - by NAME, in the CodeMirror keymap (`key: "Mod-s"`),
which no pattern looking for a comparison against `"s"` will ever see. The fix
that reading asked for was a second handler on the window, which would have saved
TWICE per keystroke, the second carrying the stamp the first had just replaced -
so the editor would have answered its own save with "this file changed on disk".
A matcher that does not know a form does not find a hole; it manufactures one,
and the repair it asks for is worse than the thing it reported.

WHAT COUNTS. A file that registers a menu (`menu_register`) and gives an item a
`shortcut`. The last `+`-separated segment is the key; the modifiers are not
checked, because an app that compares the key at all is an app that has thought
about the binding, and asking a text matcher to agree about `ctrlKey` ordering
buys a false red rather than a finding.

HOW A BINDING IS RECOGNISED, and it is deliberately GENEROUS. Any comparison
against the key as a literal, in an app file that has a keydown handler; a `case`
label; the `KeyX` code form; or an editor keymap binding it by name
(`key: "Mod-s"`). The key is regularly held in a local first - the calendar reads `const k = e.key` and then compares `k`, the terminal
`const key = e.key.toLowerCase()` - so a pattern anchored on `.key ===` reports
correct code. A first cut did exactly that and called seven bound calendar keys
unbound. A gate that goes red on correct code teaches people to pass
`--no-verify`, so the matcher errs toward quiet and still caught both real ones.

THE `+` KEY IS NOT A SEPARATOR. The pdf reader binds `+` for zoom and declares
it as `shortcut: "+"`. Splitting that on `+` leaves an empty key, which matches
nothing and reads as unbound. Same first cut, second false red.

WHAT IT CANNOT SEE. A key bound in a shared kit component rather than the app,
and a binding assembled from a variable (`e.key === wanted`). Both would read as
unbound; neither exists in the tree today. And the mirror of the CodeMirror
lesson: any OTHER library that binds by a name this does not know would read the
same way, so a red here is worth one grep before it is worth a handler.

Run: dev/scripts/check-menu-shortcuts-bound.py [root]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

#: Where a frontend lives.
SOURCES = ("apps/*/src", "sdk/ui-kit/src")

#: The call that publishes a menu into the shell. A file without it is not
#: making the claim.
REGISTERS = "menu_register"

#: A `shortcut` in the object form, and as the third argument of the `item()`
#: helper the apps share.
SHORTCUT_FIELD = re.compile(r'shortcut:\s*"([^"]+)"')
SHORTCUT_ARG = re.compile(r'item\(\s*[^,]+,\s*"[^"]+",\s*"([^"]+)"')

#: What a key is called in a menu versus in a `KeyboardEvent`.
ALIAS = {"del": "delete", "esc": "escape", "return": "enter", "plus": "+", "space": " "}


def key_of(shortcut: str) -> str:
    """The key a shortcut names, with the modifiers dropped.

    `+` is a key as well as the separator, so a trailing empty segment means the
    shortcut ended ON a plus rather than before one.
    """
    parts = shortcut.split("+")
    last = parts[-1]
    if last == "" and len(parts) > 1:
        return "+"
    return last


def bound(blob: str, key: str) -> bool:
    """Does this app compare a key against `key` anywhere?"""
    k = ALIAS.get(key.lower(), key.lower())
    e = re.escape(k)
    if re.search(rf'''\bcase\s+["']{e}["']''', blob, re.I):
        return True
    if re.search(rf'''["']Key{re.escape(k.upper())}["']''', blob):
        return True
    # An editor keymap binds by NAME, not by comparing an event: CodeMirror takes
    # `key: "Mod-s"`, and the text editor's Save has been bound that way since the
    # buffer was written. A first cut of this check did not know the form, called
    # `Ctrl+S` unbound, and the fix it asked for was a SECOND handler on the
    # window - two saves per keystroke, the second carrying the stamp the first
    # had already replaced, so the editor would have answered its own save with
    # "this file changed on disk". A matcher that does not know a form does not
    # find a hole, it manufactures one.
    if re.search(rf'''key:\s*["'](?:[A-Za-z]+-)*{e}["']''', blob, re.I):
        return True
    if "keydown" not in blob and "onkeydown" not in blob:
        return False
    return bool(re.search(rf'''[!=]==?\s*["']{e}["']''', blob, re.I))


def app_of(path: Path) -> Path:
    """The frontend root a file belongs to, so the search stays in its app."""
    for pattern in SOURCES:
        for base in ROOT.glob(pattern):
            if base in path.parents:
                return base
    return path.parent


def frontend_files(base: Path, exclude: Path) -> str:
    out = []
    for p in sorted(base.rglob("*")):
        if p.suffix not in (".ts", ".svelte"):
            continue
        if "node_modules" in p.parts or ".test." in p.name or p == exclude:
            continue
        out.append(p.read_text(encoding="utf-8", errors="ignore"))
    return "\n".join(out)


def main() -> int:
    menus: list[Path] = []
    for pattern in SOURCES:
        for base in ROOT.glob(pattern):
            for p in base.rglob("*"):
                if p.suffix in (".ts", ".svelte") and "node_modules" not in p.parts:
                    if ".test." in p.name:
                        continue
                    if REGISTERS in p.read_text(encoding="utf-8", errors="ignore"):
                        menus.append(p)

    if not menus:
        print(f"check-menu-shortcuts-bound: no menu registrations under {ROOT}", file=sys.stderr)
        return 2

    findings: list[str] = []
    declared = 0
    for menu in sorted(menus):
        text = menu.read_text(encoding="utf-8", errors="ignore")
        shortcuts = sorted(set(SHORTCUT_FIELD.findall(text)) | set(SHORTCUT_ARG.findall(text)))
        if not shortcuts:
            continue
        base = app_of(menu)
        blob = frontend_files(base, menu)
        for s in shortcuts:
            declared += 1
            if not bound(blob, key_of(s)):
                rel = menu.relative_to(ROOT)
                findings.append(
                    f"  - {rel}: the menu says `{s}` and nothing under "
                    f"{base.relative_to(ROOT)} compares a key against `{key_of(s)}`"
                )

    if findings:
        print("A menu advertises a keystroke its app never learnt:\n")
        print("\n".join(findings))
        print(
            "\nBind the key, or take the accelerator off the menu item. The text "
            "beside a menu label is a promise about the keyboard, and the only "
            "way anybody finds out it was empty is by pressing it."
        )
        return 1

    print(
        f"{len(menus)} menu registration(s), {declared} advertised keystroke(s); "
        "each one is compared somewhere in its app."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
