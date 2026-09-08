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

WHAT COUNTS AS CALLED: the command's name appears as a string literal in THAT
APP's own frontend, or in the shared kit every app bundles. Generous about the
SHAPE (a name can be built into a table or passed to a helper, so demanding the
literal `invoke("x")` would report live commands as dead) and strict about the
APP, because a Tauri command does not cross an app boundary - a frontend can only
invoke its own backend's handler.

That second half was pooled across every frontend until 8 September, and the
pooling hid real findings. `night_light_set` is registered by BOTH the shell and
Settings and called from Settings' own page; pooled, that one call vouched for
the shell's copy too. `register_menu` is the sharper case: harness declares its
menu to its own backend, and pooling read that as proof the SHELL's global-menu
registration is used, when nothing in the shell's frontend touches it. Per-app
scoping raised the count from 49 to 70, and every one of the 21 is a command no
surface of its own app can reach.

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
    "desktop-shell": (19, "the 8 September scan, re-measured per-app, less the three night-light commands, `get_project` (kept as the function `intent_ipc` calls) and `save_shell_config` (deleted - a whole-file write nothing called, and the read-modify-write its neighbour warns loses data) (`night_light_set` kept as the function `quick_action_run` calls, the schedule and location setters deleted - Settings owns those controls and reaches the compositor through the shell.toml watcher, which no Tauri command can do across apps). Three clusters: the six `qs_layout_*` writers (a SECOND writer for a file Settings already edits through `config_set` - which side owns the layout's invariants is a decision, not a deletion), the global-menu registry (`register_menu`/`set_menu_state`/`unregister_menu` - the shell's cross-app menu path, which no app uses because each declares its menu to its own backend), and features with no UI at all"),
    "files": (2, "the 8 September scan"),
    "harness": (16, "the 8 September scan; arlen-ui's app, so theirs to answer, `frontend_log` included - the marker is per-app, so ours on the other six does not answer for theirs"),
    "settings": (13, "the 8 September scan, re-measured per-app, less `theme_get`, `keybindings_get_defaults` and `keybindings_get_all_conflicts` (all deleted - each a second name for something a live command already returns). The three `extensions_*` commands are the backend of the management surface the shell-extension strand named, so a strand rather than a loose end; four theme readers; and the ai/keybinding readers"),
    "system-monitor": (1, "the 8 September scan"),
    "store": (4, "the 8 September scan; arlen-ui's app, so theirs to answer, `frontend_log` included - I do not write in their tree, so the marker the other apps carry is not mine to add here"),
    "terminal": (1, "the 8 September scan"),
}

# A command explains its own absence with this in its doc comment, followed by
# the reason. Explicit rather than a fuzzy match on "no caller today": a marker a
# reader can grep is worth more than a phrase a scanner guesses at.
SELF_EXCUSED = re.compile(r"NO CALLER:")

# The shared kit is bundled by every app, so a name it carries is callable from
# any of them. An app's own `src/` is the rest of its call set.
SHARED_FRONTEND = ("sdk/ui-kit",)
FRONTEND_SUFFIXES = (".ts", ".svelte", ".js")
SKIP = ("/node_modules/", "/build/", "/.svelte-kit/", "/target/")


def frontend_strings(root: Path, bases: tuple[str, ...]) -> set[str]:
    """Every string literal under `bases`, as the call set for one app."""
    names: set[str] = set()
    for base in bases:
        if not (root / base).is_dir():
            continue
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


def self_excused(root: Path, app: str) -> set[str]:
    """Commands in THIS app whose own doc comment says why nothing calls them.

    Per-app for the same reason the call set is: a name is not one command. Seven
    apps register a `frontend_log` and each is its own; a marker written on one
    must not answer for the other six, or the marker becomes the pooling bug this
    check was just corrected for.
    """
    out: set[str] = set()
    for src in (root / "apps" / app / "src-tauri" / "src").rglob("*.rs"):
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
    shared = frontend_strings(root, SHARED_FRONTEND)
    answered = 0

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
        excused = self_excused(root, app)
        answered += len(excused)
        called = frontend_strings(root, (f"apps/{app}/src",)) | shared | excused
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
    print(
        f"check-commands-invoked: {total} command(s) across {apps} app(s), "
        f"{carried} carried, {answered} answered where they are written"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
