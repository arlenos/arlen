#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that an app may publish what its shell grants let it publish.

Two grants stand between an app and a shell surface, and they are written in
different files by different hands. The Tauri capability
(`src-tauri/capabilities/default.json`) decides whether the webview may CALL
`ambient.set` at all; it is compiled into the binary. The event-bus publish
scope in the app's permission profile decides whether the event that call
produces is DELIVERED. Grant the first and forget the second and the app calls
a command that succeeds, the SDK emits, and the bus drops it.

WHY IT IS SILENT TODAY, which is the whole reason this is a check and not a bug
report. The bus runs in shadow mode (`ARLEN_EVENT_BUS_ENFORCE`, off by default):
an ungranted publish is logged at debug and delivered anyway. So every one of
these is invisible until the enforce cutover, at which point a badge, a menu or
a tint stops appearing with nothing in the app to say why.

WHAT THE SWEEP FOUND on 9 September, before this existed: seven apps, eleven
topics. The terminal could call `ambient.set` and not publish it - a producer
that had landed the same afternoon WITH its Tauri capability and without this
half. Five apps could register a top-bar menu they were not granted, which the
files profile had already named as the trap: "the app has no emitter to grep
for", because the plugin emits from the app's process on its behalf.

THE MAPPING IS EXACT, which is what makes this checkable rather than a judgement.
One plugin command emits one topic, both sides written in `sdk/os-sdk/src/*.rs`,
so the table below is a transcription rather than an opinion. A command that
gains a topic, or a surface that gains a command, has to be added here - and
`test-check-publish-grants.mjs` fails if the table names a command the plugin
does not have.

WHAT IT DOES NOT COVER, and why that half is a sweep rather than a rule. Daemons
and the shell publish too, and they have no capability file to diff against - the
only source for what they emit is the Rust, and a scan of `.emit("...")` misses
the ones that write the envelope to the producer socket by hand, which is how
the shell emits `permission.changed`. A gate that silently skips a topic gives
worse than no answer. Swept by hand on 9 September instead: every daemon with a
shipped profile (auditd, calendard, code-indexer, knowledge, modulesd, powerd)
and the shell declare everything they emit, so the class was app-side only.

WHAT IT DOES NOT CHECK. Whether the app SHOULD have the grant: that is the
enrollment decision, and an app granted a surface it never uses is
`check-plugin-surfaces-reached`'s question. This one only says the two halves
agree.

Run: dev/scripts/check-publish-grants.py [root]
"""

import json
import re
import sys
import tomllib
from pathlib import Path

#: Plugin capability suffix -> the topic that command's SDK surface emits.
#: Transcribed from `sdk/os-sdk/src/<surface>.rs`; the control walks the plugin's
#: own permission list to catch a name that no longer exists.
TOPIC: dict[str, str] = {
    "presence-set": "app.presence.set",
    "presence-clear": "app.presence.clear",
    "timeline-record": "app.timeline.record",
    "badges-set": "app.badge.set",
    "badges-clear": "app.badge.cleared",
    "ambient-set": "app.ambient.set",
    "ambient-clear": "app.ambient.cleared",
    "shortcuts-register": "app.shortcut.register",
    "shortcuts-set-state": "app.shortcut.state_changed",
    "shortcuts-clear": "app.shortcut.cleared",
    "menu-register": "app.menu.registered",
    "menu-unregister": "app.menu.unregistered",
    "toolbar-set-quick-actions": "app.toolbar.quick_actions",
    "toolbar-set-breadcrumb": "app.toolbar.breadcrumb",
    "toolbar-set-progress": "app.toolbar.progress",
    "toolbar-clear-progress": "app.toolbar.progress_cleared",
    "toolbar-clear": "app.toolbar.cleared",
    "annotation-set": "app.annotation.set",
    "annotation-clear": "app.annotation.cleared",
}

#: Apps whose shipped profile is knowingly behind, with why. EMPTY: the seven the
#: first sweep found were all fixed the day it landed, and an entry here is a
#: surface somebody decided to leave dark at the cutover.
CARRIED: dict[str, str] = {}

GRANT = re.compile(r'"arlen-shell:allow-([a-z-]+)"')


def declared_publish(profile: Path) -> set[str]:
    """The topics a profile's `[event_bus] publish` list names.

    Parsed as TOML rather than matched. The first cut of this used a regex over
    the section and reported four apps that were correctly granted - a comment
    inside the list is enough to end a lazy block match, and the file is a
    parseable format, so there is no reason to guess at it.
    """
    try:
        doc = tomllib.loads(profile.read_text(errors="replace"))
    except tomllib.TOMLDecodeError:
        # An unparseable profile is `check-profile-catalogue`'s finding; saying
        # it again here would report one fault as two.
        return set()
    published = doc.get("event_bus", {}).get("publish", [])
    return {t for t in published if isinstance(t, str)}


def covered(topic: str, declared: set[str]) -> bool:
    """Whether `topic` is granted, honouring a trailing `*` in a declared entry.

    A profile may write `app.menu.*` rather than both halves, and that is a real
    grant rather than a shorthand - the bus matches it by prefix. Reading it as a
    literal would report an app that is correctly granted.
    """
    for entry in declared:
        if entry == topic:
            return True
        if entry.endswith("*") and topic.startswith(entry[:-1]):
            return True
    return False


def main() -> int:
    root = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
    profiles = root / "dev/mkosi/mkosi.extra/var/lib/arlen/permissions/1000"
    caps = sorted((root / "apps").glob("*/src-tauri/capabilities/default.json"))
    if not caps or not profiles.is_dir():
        print("check-publish-grants: no apps or no shipped profiles, so the scan is pointed wrong")
        return 1

    checked = 0
    problems: list[str] = []
    for cap in caps:
        app = cap.parts[-4]
        granted = {TOPIC[g] for g in GRANT.findall(cap.read_text(errors="replace")) if g in TOPIC}
        if not granted:
            continue
        conf = json.loads((cap.parent.parent / "tauri.conf.json").read_text(errors="replace"))
        app_id = conf.get("identifier", "")
        profile = profiles / f"{app_id}.toml"
        if not profile.is_file():
            # No shipped profile at all is `check-app-profiles`' question, not
            # this one: reporting it here would say the same thing twice.
            continue
        checked += 1
        declared = declared_publish(profile)
        missing = sorted(t for t in granted if not covered(t, declared))
        if missing and app not in CARRIED:
            problems.append(
                f"  - {app} ({app_id}): may call the surface, may not publish it.\n"
                f"    {', '.join(missing)}"
            )

    if problems:
        print("Shell surfaces an app is granted and its profile does not carry:\n")
        print("\n".join(problems))
        print(
            "\n  The Tauri capability lets the webview call it; the profile's"
            "\n  `[event_bus] publish` decides whether the event is delivered. Until"
            "\n  the enforce cutover the bus logs and delivers anyway, so this costs"
            "\n  nothing today and costs the whole surface on the day it flips."
        )
        return 1

    print(
        f"check-publish-grants: {checked} app(s) with a shipped profile; "
        "each may publish every shell surface it was granted"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
