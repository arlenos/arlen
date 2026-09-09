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

NOT COVERED: Tauri PLUGIN commands. A plugin registers its own handler set and is
invoked as `plugin:<name>|<cmd>`, through a TypeScript binding the plugin ships
(`sdk/tauri-plugin-*/index.ts`) which builds that string in a template literal. The
same question is askable there and this scan cannot answer it - the call from an
app is a function call into the binding, not a name in the app's source. Measured
once on 8 September: reading the plugin handler lists alone made 40 commands look
dark and every one was a false positive, because the binding calls them all. The
gap is named here rather than closed by widening the check badly.

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
    "desktop-shell": (
        6,
        "the six `qs_layout_*` writers, and nothing else. They are a second writer "
        "for a file Settings already edits, so which side owns the layout "
        "invariants is a decision rather than a deletion. Everything else the "
        "8 September scan found here is answered: `connect_hidden_network` (the "
        "network panel joins a hidden one), `undo_detail` (the recent-actions "
        "panel opens a row's record), `app_shortcut_invoke` (the launcher lists "
        "the focused app's actions) and `notification_get_known_apps` (the "
        "history reads one app at a time) got their surfaces; "
        "`waypointer_list_plugins`, `register_menu`, `unregister_menu`, "
        "`set_menu_state` and `modulesd_set_enabled` were deleted, each a second "
        "path to something a live one already reaches"
    ),
    "harness": (16, "the 8 September scan; arlen-ui's app, so theirs to answer, `frontend_log` included - the marker is per-app, so ours on the other six does not answer for theirs"),
    "settings": (
        4,
        "the model picker's backend - `ai_models_list`, `ai_defaults_set` and the "
        "`ai_uncensored_*` opt-in pair - in arlen-ui's lane, and that page renders "
        "a fixture today, so it is mid-build rather than missing. The three "
        "`extensions_*` came out on 9 September when the management surface landed "
        "and called them, which is what the number dropping is for. Everything the "
        "8 September scan found is now answered: `theme_get`, "
        "`keybindings_get_defaults`, `keybindings_get_all_conflicts` and "
        "`ai_activity_recent` deleted; `settings_app_audit`, "
        "`theme_contrast_report`, `revoke_consent`, `theme_resolved_sounds` and "
        "`theme_resolved_terminal` given their readers - the last two were a "
        "hardcoded copy of the theme's own values sitting where the resolver's "
        "answer belonged"
    ),
    "store": (4, "the 8 September scan; arlen-ui's app, so theirs to answer, `frontend_log` included - I do not write in their tree, so the marker the other apps carry is not mine to add here"),
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
