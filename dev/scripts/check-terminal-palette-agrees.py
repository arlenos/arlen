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

**The sixteen were not the whole palette.** `fg`, `bg` and `cursor` are the three
the theme does NOT author: `resolve_terminal` synthesises them from `fg.primary`,
`bg.app` and `accent`, which is what the emitted kitty, foot, Alacritty and
Xresources configs carry. The in-app grid is a copy of those too, and on 15
September it had drifted on two of the three - its text was `#e4e5ea` against the
theme's `#fafafa`, its cursor the `accent_pressed` grey against the accent - so
the Arlen terminal painted dimmer text than every other terminal on the same
machine, and the sixteen-slot check said the palette agreed. A gate that covers
most of a thing is read as covering the thing.

These three are compared against their SOURCE tokens, and an authored
`[terminal]` block wins over the synthesis exactly as the resolver lets it, so
authoring one there stays the way to move all five surfaces at once.
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


#: The three the resolver synthesises, and the token each comes from.
#: `sdk/theme` `resolve_terminal`: fg <- fg.primary, bg <- bg.app, cursor <- accent.
SYNTHESISED = {
    "fg": ("color.fg", "primary"),
    "bg": ("color.bg", "app"),
    "cursor": ("color.semantic", "accent"),
}

#: What each copy calls them. A copy that does not hold a slot simply omits it -
#: the Settings floor has no cursor field, and inventing one to satisfy a gate
#: would be the gate changing the code rather than checking it.
COPY_KEYS = {
    "apps/terminal/src/lib/terminal-theme.ts": {
        "fg": "foreground",
        "bg": "background",
        "cursor": "cursor",
    },
    "apps/settings/src/lib/stores/themeSystem.ts": {
        "fg": "termFg",
        "bg": "termBg",
    },
}


def token(text: str, table: str, key: str) -> str | None:
    """One `[table] key = "#hex"` value out of the theme file."""
    block = re.search(rf"\[{re.escape(table)}\]\n(.*?)(?=\n\[|\Z)", text, re.S)
    if not block:
        return None
    found = re.search(r'\b' + re.escape(key) + r'\s*=\s*"(#[0-9a-fA-F]+)"', block.group(1))
    return found.group(1).lower() if found else None


def synthesised_sources(text: str) -> dict[str, str]:
    """What fg/bg/cursor resolve to: an authored `[terminal]` value, else the token."""
    authored = {}
    block = re.search(r"\[terminal\]\n(.*?)(?=\n\[|\Z)", text, re.S)
    if block:
        pairs = re.findall(r'(\w+)\s*=\s*"(#[0-9a-fA-F]+)"', block.group(1))
        authored = {k: v.lower() for k, v in pairs}
    out = {}
    for slot, (table, key) in SYNTHESISED.items():
        value = authored.get(slot) or token(text, table, key)
        if value:
            out[slot] = value
    return out


def copy_value(text: str, key: str) -> str | None:
    """A `key: "#hex"` entry in one of the TypeScript copies."""
    found = re.search(r'\b' + re.escape(key) + r'\s*:\s*"(#[0-9a-fA-F]+)"', text)
    return found.group(1).lower() if found else None


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

    theme_text = THEME.read_text(encoding="utf-8", errors="replace")
    synthesised = synthesised_sources(theme_text)
    if len(synthesised) < len(SYNTHESISED):
        absent = [s for s in SYNTHESISED if s not in synthesised]
        print(
            "!! NOTHING TO COMPARE AGAINST: the theme resolves no source for "
            f"{', '.join(absent)}, so the copies have nothing to agree with",
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
        text = path.read_text(encoding="utf-8", errors="replace")
        copies.append((path, reader(text), what, text))

    problems = []
    checked_synth = 0
    for path, got, what, text in copies:
        rel = path.relative_to(ROOT)
        for slot in SLOTS:
            want = authored[slot]
            if slot not in got:
                problems.append(f"{rel}: {what} names no {slot}; the theme authors it as {want}")
            elif got[slot] != want:
                problems.append(f"{rel}: {slot} is {got[slot]}, the theme says {want}")
        for slot, key in COPY_KEYS.get(str(rel), {}).items():
            want = synthesised[slot]
            have = copy_value(text, key)
            checked_synth += 1
            if have is None:
                problems.append(
                    f"{rel}: {what} names no `{key}`; the resolver gives every other "
                    f"terminal {want} for {slot}"
                )
            elif have != want:
                problems.append(
                    f"{rel}: `{key}` is {have}, the resolver synthesises {slot} as {want} "
                    f"from {'.'.join(SYNTHESISED[slot])}"
                )

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
        f"{len(copies)} copy(ies) downstream, plus {checked_synth} synthesised "
        "value(s) agreeing with the tokens they come from."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
