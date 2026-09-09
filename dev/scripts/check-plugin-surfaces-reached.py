#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Check that every command the shell plugin registers is reached by an app.

THE BLIND SPOT THIS CLOSES. `check-commands-invoked` asks the same question one
app at a time, over each app's OWN `src-tauri`. The shell plugin is not any
app's: it lives in `sdk/tauri-plugin-shell`, is linked into all eighteen, and
registers twenty-eight commands that no per-app scan can see. So a whole
app-facing surface can be built, documented, consumed at the far end, and reached
by nobody, and every existing check stays green.

Measured on 9 September: of twenty-eight, six are reached. The other
twenty-two are the census below - and they are not scaffolding, because both
consumer ends are real:

  * The SHELL draws badges, ambient effects and the focused app's shortcut list
    (`apps/desktop-shell/src/lib/stores/appStateStores.ts`, subscribed in
    `event_bus.rs`, each with its own `*-api.md`). Nothing publishes one.
  * The KNOWLEDGE DAEMON promotes `app.presence.*`, `app.timeline.record` and
    `app.annotation.*` into graph nodes (`promotion.rs`). Nothing publishes one
    either, so the app-level half of what the graph could know is dark and only
    the kernel sensor's view of a session reaches it.

WHAT COUNTS AS REACHING ONE. A direct `plugin:arlen-shell|<command>` invoke, or a
call through the TS wrapper (`toolbar.setBreadcrumb(...)`), from any app frontend
or from the KIT - a shared control's calls count as the app's, the same rule
`check-window-grants` follows, and `theme_get` is reached exactly that way.

WHY A CARRIED CENSUS RATHER THAN A RED. Whether an unreached command is a feature
somebody has to finish, a second path to something a live one already does, or a
delete is the planner's call, and a check that goes red on twenty-two of them
teaches people to skip it. So each is carried WITH what is true about it today,
and two rules keep it a queue rather than a hole: a NEW unreached command fails,
and a carried one that GAINS a producer fails until its line comes out. Mail took
the badge line out on 9 September by publishing its unread count.

Run: dev/scripts/check-plugin-surfaces-reached.py [root]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

PLUGIN_LIB = "sdk/tauri-plugin-shell/src/lib.rs"
PLUGIN_TS = "sdk/tauri-plugin-shell/index.ts"

#: Where a producer may live. The kit is included because a control it owns calls
#: on every app's behalf.
PRODUCERS = ("apps/*/src", "sdk/ui-kit/src")

#: The shell is the CONSUMER of these events; its own calls are not an app
#: reaching the surface.
NOT_A_PRODUCER = "desktop-shell"

#: Command to what is true about it today. Written from the consumer end in each
#: case, because "nobody calls it" is not the interesting half.
CARRIED: dict[str, str] = {
    # ── The shell draws these and nothing fills them ──────────────────
    "ambient_set": "the shell keeps a per-app ambient effect (ambient-api.md); no app publishes one",
    "ambient_clear": "pairs with ambient_set",
    "shortcuts_register": "the shell keeps the focused app's shortcut list and the waypointer lists it (shortcuts-api.md, app_shortcuts.rs); no app registers one",
    "shortcuts_set_state": "the diff update for a registered list, so it waits on the register",
    "shortcuts_clear": "pairs with shortcuts_register",
    "toolbar_set_progress": "the toolbar's progress bar; the file manager draws its own progress zone instead, which is the second-path question rather than a hole",
    "toolbar_clear_progress": "pairs with toolbar_set_progress",
    "toolbar_clear": "nothing takes its toolbar down; the two producers replace theirs by setting it again",
    "menu_unregister": "nothing takes its menu down either, for the same reason - a re-register replaces the tree, and a menu is only drawn while its app is focused",
    # ── The knowledge daemon promotes these and nothing sends them ────
    "presence_set": "promoted into a UserAction node by the knowledge daemon (promotion.rs `app.presence.set`); no app says what it is doing",
    "presence_clear": "pairs with presence_set, and the daemon promotes it too",
    "timeline_record": "promoted by the knowledge daemon (`app.timeline.record`); no app records anything",
    "annotation_set": "promoted into an Annotation node (`app.annotation.set`); no app annotates",
    "annotation_clear": "pairs with annotation_set, promoted as `app.annotation.cleared`",
    "annotation_get": "the read side of the same surface",
    "annotation_subscribe_prepare": "one third of `annotations.onChanged`, which nothing subscribes to",
    "annotation_subscribe_start": "one third of `annotations.onChanged`",
    "annotation_unsubscribe": "one third of `annotations.onChanged`",
    # ── Inert on purpose, and the only one that is ────────────────────
    #
    # I called this the deletion candidate before reading the module, which was
    # the wrong way round. `os_sdk::spatial` is a DELIBERATE forward-compatible
    # stub: it quotes the foundation paper - "Until then, `shell.spatial` calls
    # are accepted and silently ignored" - and exists so an app can declare a
    # placement hint today and need no source change when the compositor
    # extension lands. `Spatial::hint` does not even emit; the module says which
    # topic it will use when it does. Its consumer is the compositor, which is a
    # separate repo with its own agent, so "no consumer in this tree" was never
    # evidence of anything.
    "spatial_hint": "an intentional inert stub (os_sdk/spatial.rs quotes the paper: calls are accepted and silently ignored until the compositor extension lands, which is a separate repo); it does not emit, so there is nothing here to reach",
}


def registered(lib: str) -> list[str]:
    block = re.search(r"generate_handler!\[(.*?)\]", lib, re.S)
    if not block:
        return []
    return [m.split("::")[-1] for m in re.findall(r"([A-Za-z_:]+)\s*,", block.group(1))]


def wrapper_map(ts: str) -> dict[str, set[str]]:
    """`command -> {object.method}` read out of the TS wrapper's own bodies."""
    out: dict[str, set[str]] = {}
    obj = method = None
    for line in ts.splitlines():
        m = re.match(r"export const (\w+) = \{", line)
        if m:
            obj = m.group(1)
            continue
        m = re.search(r"async (\w+)\s*[(<]", line)
        if m:
            method = m.group(1)
            continue
        m = re.search(r"\$\{PLUGIN\}\|(\w+)", line)
        if m and obj and method:
            out.setdefault(m.group(1), set()).add(f"{obj}.{method}")
    return out


def sources() -> list[tuple[str, str]]:
    out = []
    for pattern in PRODUCERS:
        for base in ROOT.glob(pattern):
            if NOT_A_PRODUCER in base.parts:
                continue
            owner = base.relative_to(ROOT).parts[1]
            for p in base.rglob("*"):
                if p.suffix in (".ts", ".svelte") and "node_modules" not in p.parts:
                    out.append((owner, p.read_text(encoding="utf-8", errors="ignore")))
    return out


def main() -> int:
    lib_path, ts_path = ROOT / PLUGIN_LIB, ROOT / PLUGIN_TS
    if not lib_path.is_file() or not ts_path.is_file():
        print(f"check-plugin-surfaces-reached: no shell plugin under {ROOT}", file=sys.stderr)
        return 2

    commands = registered(lib_path.read_text(encoding="utf-8"))
    if not commands:
        print("check-plugin-surfaces-reached: the plugin registers nothing", file=sys.stderr)
        return 2

    wrappers = wrapper_map(ts_path.read_text(encoding="utf-8"))
    texts = sources()
    if not texts:
        print(f"check-plugin-surfaces-reached: no producer sources under {ROOT}", file=sys.stderr)
        return 2

    unreached: list[str] = []
    answered: list[str] = []
    reached = 0
    for command in commands:
        who: set[str] = set()
        for owner, text in texts:
            if f"arlen-shell|{command}" in text:
                who.add(owner)
                continue
            for call in wrappers.get(command, ()):
                obj, method = call.split(".")
                if obj in text and re.search(rf"\b{re.escape(obj)}\.{re.escape(method)}\s*\(", text):
                    who.add(owner)
        if who:
            reached += 1
            if command in CARRIED:
                answered.append(
                    f"  - {command}: reached now by {', '.join(sorted(who))}, and the census "
                    f"still says it is not."
                )
        elif command not in CARRIED:
            unreached.append(
                f"  - {command}: the plugin registers it and no app or kit control calls it. "
                f"Either something reaches it, or it goes in this check's census with what is "
                f"true about it - who consumes it, and whether that end is built."
            )

    stale = [c for c in CARRIED if c not in commands]
    if stale:
        print("The census names a command the plugin no longer registers:\n")
        for c in sorted(stale):
            print(f"  - {c}")
        print("\nTake it out; a census that outlives its subject is not a record of anything.")
        return 1

    if answered:
        print("A carried surface has a producer now:\n")
        print("\n".join(answered))
        print(
            "\nTake the entry out. A census that keeps a line about something "
            "somebody already did is where the next unreached one hides."
        )
        return 1

    if unreached:
        print("A shell-plugin command nothing reaches, and nothing says why:\n")
        print("\n".join(unreached))
        return 1

    print(
        f"{len(commands)} shell-plugin command(s); {reached} reached by an app or the kit, "
        f"{len(CARRIED)} carried with what is true about them."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
