#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Check that every registered Tauri command is one some surface actually calls.

WHY THIS EXISTS. `check-invoke-exists.py` reads this boundary in one direction: a
frontend that invokes a command no host registers, which throws where a person can
see it. The other direction is silent. A command that is registered, implemented,
tested and never invoked is a feature that exists everywhere except in front of
somebody - and it looks finished from both ends, which is why it survives.

That is not hypothetical. On 8 September `waypointer_search` turned out to be the
only command in the shell that asks the module runtime for results, and nothing
called it: the whole Tier 1 half of the launcher could work perfectly and never
put a row on a screen. The first scan for the class found fifty-two more.

WHAT COUNTS AS CALLED, and it is deliberately generous: the command's name appears
as a string literal ANYWHERE in any app frontend or in the shared kit. Not
`invoke("x")` specifically, because a name can be built into a table, passed to a
helper or shared between two apps that both register it - and a scanner that
demanded the literal call shape would report live commands as dead. The cost of
being generous is that a name mentioned only in a comment counts; the benefit is
that every finding is real.

TWO WAYS TO BE EXCUSED, and the first is the one to prefer. A command whose own
doc comment carries a `NO CALLER:` line and a reason is answered where a reader
will find it - beside the code, in the same place they would go to ask. The
carried list below is for the rest, and it MAY SHRINK and MAY NOT GROW: an entry
there is a claim that a command is deliberately unreachable, which is a thing to
decide rather than to inherit. A central table of fifty reasons is a table nobody
reads; the marker is how an entry leaves it.

Shown to fail before being trusted: `dev/scripts/test-check-commands-invoked.mjs`.

Usage: check-commands-invoked.py [repo-root]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

# Commands nobody calls, by app, with why. The first scan for this class found
# these and none of them has been answered yet; "not triaged" is an honest note
# and a fabricated per-command justification would be worse than none. Each wants
# the same answer: call it, or delete it.
CARRIED: dict[str, tuple[int, str]] = {
    "desktop-shell": (17, "the 8 September scan, less `waypointer_search` and the two toggle commands `quick_action_run` had already superseded. What is left splits three ways: the six `qs_layout_*` writers (a SECOND writer for a file Settings already edits through `config_set` - which side owns the layout's invariants is a decision, not a deletion), the three-command global-menu registry (nothing registers a menu), and features with no UI at all"),
    "files": (2, "the 8 September scan"),
    "harness": (10, "the 8 September scan; arlen-ui's app, so theirs to answer"),
    "settings": (14, "the 8 September scan, less `theme_list_fonts` which answered for itself and `theme_set_accent` which was deleted with the second accent channel. What is left: the three `extensions_*` commands (the backend of the management surface the shell-extension work named, so a strand rather than a loose end), four theme readers, and the keybinding and ai readers"),
    "store": (2, "the 8 September scan; arlen-ui's app, so theirs to answer"),
    "system-monitor": (1, "the 8 September scan"),
}

# A command explains its own absence with this in its doc comment, followed by
# the reason. Explicit rather than a fuzzy match on "no caller today": a marker a
# reader can grep is worth more than a phrase a scanner guesses at.
SELF_EXCUSED = re.compile(r"NO CALLER:")

FRONTEND_ROOTS = ("apps", "sdk/ui-kit")
FRONTEND_SUFFIXES = (".ts", ".svelte", ".js")
SKIP = ("/node_modules/", "/build/", "/.svelte-kit/", "/target/")


def frontend_strings(root: Path) -> set[str]:
    """Every string literal in every frontend source, as the generous call set."""
    names: set[str] = set()
    for base in FRONTEND_ROOTS:
        for path in (root / base).rglob("*"):
            if path.suffix not in FRONTEND_SUFFIXES:
                continue
            if any(s in str(path) for s in SKIP):
                continue
            try:
                names |= set(re.findall(r'"([a-z0-9_]+)"', path.read_text(errors="replace")))
            except OSError:
                continue
    return names


def self_excused(root: Path) -> set[str]:
    """Commands whose own doc comment says why nothing calls them."""
    out: set[str] = set()
    for src in (root / "apps").rglob("src-tauri/src/**/*.rs"):
        text = src.read_text(errors="replace")
        for m in re.finditer(r"#\[tauri::command\][^\n]*\n\s*(?:pub\s+)?(?:async\s+)?fn\s+(\w+)", text):
            # The doc block immediately above the attribute.
            head = text[: m.start()].rsplit("\n\n", 1)[-1]
            if SELF_EXCUSED.search(head):
                out.add(m.group(1))
    return out


def registered(lib: Path) -> list[str]:
    """The command names in this app's `generate_handler![...]`."""
    text = lib.read_text(errors="replace")
    block = re.search(r"generate_handler!\[(.*?)\]", text, re.S)
    if not block:
        return []
    out = []
    for line in block.group(1).split("\n"):
        line = line.strip().rstrip(",")
        if not line or line.startswith("//"):
            continue
        out.append(line.split("::")[-1])
    return out


def main() -> int:
    root = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
    called = frontend_strings(root) | self_excused(root)

    apps = 0
    total = 0
    problems: list[str] = []
    for lib in sorted((root / "apps").rglob("src-tauri/src/lib.rs")):
        cmds = registered(lib)
        if not cmds:
            continue
        apps += 1
        total += len(cmds)
        app = lib.relative_to(root / "apps").parts[0]
        dark = sorted(c for c in cmds if c not in called)
        allowed, why = CARRIED.get(app, (0, ""))
        if len(dark) > allowed:
            new = dark if not allowed else dark
            problems.append(
                f"  - {app}: {len(dark)} command(s) no surface names, carried as {allowed}.\n"
                f"    {', '.join(new)}"
            )
        elif len(dark) < allowed:
            problems.append(
                f"  - {app}: carried as {allowed} ({why}) and only {len(dark)} left. "
                "Lower the number so a new one cannot hide behind it."
            )

    if apps == 0:
        print("check-commands-invoked: no command handlers found, so the scan is pointed wrong")
        return 1

    if problems:
        print("Commands registered and never called:")
        print()
        print("\n".join(problems))
        print()
        print("  A command nobody invokes is a feature that exists everywhere except in")
        print("  front of somebody. Call it, or delete it.")
        return 1

    carried = sum(n for n, _ in CARRIED.values())
    excused = len(self_excused(root))
    print(
        f"check-commands-invoked: {total} command(s) across {apps} app(s), "
        f"{carried} carried, {excused} answered where they are written"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
