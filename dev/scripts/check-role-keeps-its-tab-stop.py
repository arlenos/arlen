#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that nothing claims an interactive role and then refuses focus.

`design-system.md` 6.13, written from the quick-settings tile: "A component whose
visual object performs two actions is two controls in a container. The container
is not focusable, carries no role, and exists to group. Reaching for
`tabindex="-1"` to keep a single outer control is the signal that the structure is
wrong."

This is that signal, read mechanically. An element that tells a screen reader it
is a button and then takes itself out of the tab order is a control a keyboard
cannot reach: announced, described, and unusable. The tile's detail action spent
months in exactly that state - the strip said `role="button"`, carried an
`aria-label`, and no keyboard could open it.

WHAT COUNTS: an interactive role (button, link, menuitem, tab, option, checkbox,
radio, switch) on the same element as `tabindex="-1"`.

WHAT DOES NOT: `tabindex="-1"` on its own. A container focused programmatically so
Escape reaches its handler is a real pattern and carries no role, which is exactly
the shape 6.13 asks for. A roving-tabindex widget is also not this: its items sit
under a `role` that owns the arrow keys (`listbox`, `menu`, `tablist`,
`radiogroup`, `grid`, `tree`, `toolbar`), and the container is the tab stop. Those
roles are listed below and their items are exempt - but only when a real container
role is in the file, because a lone `role="option"` with no `listbox` above it is
the defect again.

Run: dev/scripts/check-role-keeps-its-tab-stop.py [root]
"""

import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

#: Roles that make an element a control a person operates directly.
INTERACTIVE_ROLES = {
    "button", "link", "menuitem", "menuitemcheckbox", "menuitemradio",
    "tab", "option", "checkbox", "radio", "switch",
}

#: Container roles that own the arrow keys, so their items legitimately sit at
#: `tabindex="-1"` under one tab stop (the roving-tabindex pattern).
ROVING_CONTAINERS = {
    "listbox", "menu", "menubar", "tablist", "radiogroup", "grid", "tree",
    "treegrid", "toolbar", "combobox",
}

#: An opening tag, whatever its attributes span.
TAG = re.compile(r"<[a-zA-Z][^>]*>", re.S)
ROLE = re.compile(r'role\s*=\s*"([a-zA-Z]+)"')
MINUS_ONE = re.compile(r'tabindex\s*=\s*(?:"-1"|\{\s*-1\s*\}|-1\b)')

#: Elements the gate does not hold, `<path>:<role>`, with the reason.
CARRIED: dict[str, str] = {}


def offenders(source: str) -> list[tuple[int, str]]:
    """Every (line, role) in one file that claims a role and refuses focus."""
    roving = {
        m.group(1) for m in ROLE.finditer(source) if m.group(1) in ROVING_CONTAINERS
    }
    out = []
    for m in TAG.finditer(source):
        tag = m.group(0)
        role = ROLE.search(tag)
        if not role or role.group(1) not in INTERACTIVE_ROLES:
            continue
        if not MINUS_ONE.search(tag):
            continue
        # An item of a roving-tabindex widget, with its container in the file.
        if roving:
            continue
        out.append((source[: m.start()].count("\n") + 1, role.group(1)))
    return out


def main() -> int:
    roots = [ROOT / "apps", ROOT / "sdk", ROOT / "daemons"]
    files = sorted(
        p
        for root in roots
        if root.is_dir()
        for p in root.rglob("*.svelte")
        if "node_modules" not in p.parts
    )
    if not files:
        print(f"NOTHING WAS READ: no .svelte under {ROOT}", file=sys.stderr)
        return 2

    findings: list[str] = []
    for path in files:
        rel = path.relative_to(ROOT)
        for line, role in offenders(path.read_text(encoding="utf-8", errors="replace")):
            if f"{rel}:{role}" in CARRIED:
                continue
            findings.append(
                f"  - {rel}:{line}: `role=\"{role}\"` with `tabindex=\"-1\"`."
                f" A screen reader is told this is a control and a keyboard"
                f" cannot reach it. Make it a real control beside the other one"
                f" (design-system.md 6.13), or drop the role if it is not one."
            )

    if findings:
        print("A role claims a control that no keyboard can reach:", file=sys.stderr)
        for f in findings:
            print(f, file=sys.stderr)
        return 1

    print(
        f"{len(files)} component(s) read, and none of them announces a control it"
        " then takes out of the tab order."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
