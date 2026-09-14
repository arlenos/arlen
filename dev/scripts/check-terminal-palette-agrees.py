#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""The three copies of the terminal palette say the same sixteen colours.

`sdk/theme/themes/dark.toml` authors `[terminal.ansi]` for one stated reason, in
its own comment: so that the emitted GTK, Qt, foot and Xresources configs, the
xterm.js grid the terminal app paints, and the swatch editor in Settings all
show the SAME muted set. The resolver would happily synthesise a palette from the
semantic tokens; the slots are authored to overrule it, which only works if
everyone downstream reads the same sixteen values.

Two of the three are copies, and a copy of a value is a value that drifts. On 15
September the Settings floor still held a generic palette from before that
decision - its blue was `#2563eb` where the theme says `#7d9cc4` - so the page
offered a grid of colours the terminal never printed, and an axe sweep found it
by measuring a blue that fails contrast in a palette that does not.

Neither copy can simply be deleted. The Settings floor is what a failed backend
read falls back to, and blanking a swatch is worse than showing a stale one; the
terminal app paints a canvas before any theme has been projected to the frontend
and needs a palette at import time. So they stay copies, and this keeps them
true.

    drifted    a slot whose value differs from the theme file's
    missing    a slot the theme authors and a copy does not name
"""

import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
THEME = ROOT / "sdk/theme/themes/dark.toml"
TERMINAL_APP = ROOT / "apps/terminal/src/lib/terminal-theme.ts"
SETTINGS = ROOT / "apps/settings/src/lib/stores/themeSystem.ts"

# The theme file's slot names, in ANSI order. The two copies each spell them
# their own way, which is why the mapping is written out rather than derived.
SLOTS = [
    "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
    "bright_black", "bright_red", "bright_green", "bright_yellow",
    "bright_blue", "bright_magenta", "bright_cyan", "bright_white",
]


def theme_slots(text: str) -> dict[str, str]:
    """The `[terminal.ansi]` table, slot name to lowercase hex."""
    block = re.search(r"\[terminal\.ansi\]\n(.*?)(?=\n\[|\Z)", text, re.S)
    if not block:
        return {}
    found = re.findall(r"(\w+)\s*=\s*\"(#[0-9a-fA-F]+)\"", block.group(1))
    return {k: v.lower() for k, v in found}


def camel(slot: str) -> str:
    """`bright_black` as the terminal app spells it: `brightBlack`."""
    head, _, tail = slot.partition("_")
    return head + tail.capitalize() if tail else head


def terminal_app_slots(text: str) -> dict[str, str]:
    """The xterm.js ITheme object's ANSI keys."""
    found = re.findall(r"\b(bright[A-Z]\w+|black|red|green|yellow|blue|magenta|cyan|white)\s*:\s*\"(#[0-9a-fA-F]+)\"", text)
    by_camel = {k: v.lower() for k, v in found}
    return {slot: by_camel[camel(slot)] for slot in SLOTS if camel(slot) in by_camel}


def settings_slots(text: str) -> dict[str, str]:
    """`SYS_DEFAULTS`' `ansi0`..`ansi15`."""
    found = re.findall(r"\bansi(\d+)\s*:\s*\"(#[0-9a-fA-F]+)\"", text)
    by_index = {int(i): v.lower() for i, v in found}
    return {slot: by_index[i] for i, slot in enumerate(SLOTS) if i in by_index}


def main() -> int:
    if not THEME.is_file():
        print(f"!! NOTHING WAS READ: no theme file at {THEME}", file=sys.stderr)
        return 2
    authored = theme_slots(THEME.read_text(encoding="utf-8", errors="replace"))
    if len(authored) < len(SLOTS):
        absent = [s for s in SLOTS if s not in authored]
        print(
            "!! NOTHING TO COMPARE AGAINST: the theme does not author "
            f"{', '.join(absent)} in [terminal.ansi], so the copies have no source",
            file=sys.stderr,
        )
        return 2

    copies = []
    for path, reader, what in [
        (TERMINAL_APP, terminal_app_slots, "the xterm grid"),
        (SETTINGS, settings_slots, "the Settings swatch floor"),
    ]:
        if not path.is_file():
            print(f"!! NOTHING WAS READ: no copy at {path}", file=sys.stderr)
            return 2
        copies.append((path, reader(path.read_text(encoding="utf-8", errors="replace")), what))

    problems = []
    for path, got, what in copies:
        rel = path.relative_to(ROOT)
        for slot in SLOTS:
            want = authored[slot]
            if slot not in got:
                problems.append(f"{rel}: {what} names no {slot}; the theme authors it as {want}")
            elif got[slot] != want:
                problems.append(f"{rel}: {slot} is {got[slot]}, the theme says {want}")

    if problems:
        print("The terminal palette disagrees with itself:", file=sys.stderr)
        for p in problems:
            print(f"  - {p}", file=sys.stderr)
        print(
            "\nThe theme file is the source. Copy its value, or author a different one there\n"
            "so every surface follows.",
            file=sys.stderr,
        )
        return 1

    print(
        f"{len(SLOTS)} ANSI slot(s) authored in the theme and matched by "
        f"{len(copies)} copy(ies) downstream."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
