#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""An app that declares a subscribe list must still hear what its plugins subscribe.

`sdk/tauri-plugin-shell` subscribes `app.toolbar.action_invoked` and
`app.shortcut.action_invoked` from inside every Tauri app's own process
(`spawn_action_invoked_consumer`). That is how a Quick Action or a Breadcrumb
click in the shell's top bar reaches an app the shell has no webview handle for.
The subscription rides the app's connection, so the APP's profile governs it.

THE TRAP, and it caught three profiles in one day. Declaring any subscribe list
is what drops the system-tier exemption - `exempt = is_system &&
!declares_subscribe` - so the moment a profile names one topic it is held to
exactly that list, including for subscriptions its own source never mentions.
Grant `accessibility.state` and you have silently switched off the toolbar.
Reading the app's code will not save you: the plugin's subscribe is in the SDK.

IT FAILS SILENTLY. `subscribe` returns Ok, the ungranted patterns are filtered
out of the registration, and the plugin's self-healing loop waits on a receiver
that never yields. Buttons stop working; nothing anywhere says why.

The knowledge app proved it on a real enforce boot (14 Aug), which named both
patterns by app id. The terminal and the file manager had the identical defect
and were invisible only because those apps were not launched on that boot - a
check is the right shape for that, since it does not depend on which app somebody
happened to start.

WHICH APPS THIS APPLIES TO IS DERIVED, NOT LISTED: any `apps/<name>/src-tauri`
whose Cargo.toml depends on the plugin, mapped to `dev.arlen.<name>`. A hand list
would drift the first time an app added the dependency, which is exactly the
event this exists to survive.
"""

import re
import sys
import tomllib
from pathlib import Path

OWN_TREE = len(sys.argv) <= 1
REPO = Path(__file__).resolve().parents[2] if OWN_TREE else Path(sys.argv[1]).resolve()

PLUGIN = "tauri-plugin-shell"

# What the plugin subscribes on the app's behalf. Kept beside the assertion that
# the SDK still asks for exactly these, so the check cannot drift from the code.
PLUGIN_TOPICS = ("app.toolbar.action_invoked", "app.shortcut.action_invoked")

PLUGIN_SRC = "sdk/tauri-plugin-shell/src/lib.rs"
PROFILES = "dev/mkosi/mkosi.extra/var/lib/arlen/permissions/1000"


def covers(patterns: list[str], topic: str) -> bool:
    """Mirror of `pattern_matches`: exact, or a `.*` suffix on a dot boundary."""
    for p in patterns:
        if p == topic or p == "*":
            return True
        if p.endswith("*") and topic.startswith(p[:-1]):
            return True
    return False


def plugin_apps(repo: Path) -> set[str]:
    """`dev.arlen.<name>` for every app whose src-tauri links the plugin."""
    out = set()
    for cargo in repo.glob("apps/*/src-tauri/Cargo.toml"):
        if PLUGIN in cargo.read_text(encoding="utf-8", errors="replace"):
            out.add(f"dev.arlen.{cargo.parents[1].name}")
    return out


def main() -> int:
    apps = plugin_apps(REPO)
    profiles = sorted((REPO / PROFILES).glob("*.toml"))
    if not apps or not profiles:
        print(
            f"NOTHING WAS READ: {len(apps)} app(s) linking {PLUGIN} and "
            f"{len(profiles)} profile(s) under {REPO}/{PROFILES}",
            file=sys.stderr,
        )
        return 2

    # The topics are read off the SDK when it is present, so a rename there turns
    # into a red check rather than a check quietly guarding the old names. When
    # this runs against a fixture there is no SDK, and the constants stand.
    src = REPO / PLUGIN_SRC
    if src.is_file():
        text = src.read_text(encoding="utf-8", errors="replace")
        subscribed = set(re.findall(r'"(app\.[a-z_.]+)"\.to_string\(\)', text))
        if subscribed and subscribed != set(PLUGIN_TOPICS):
            print(
                f"{PLUGIN} no longer subscribes what this check guards:\n"
                f"  the SDK asks for {sorted(subscribed)}\n"
                f"  this check knows {sorted(PLUGIN_TOPICS)}\n"
                f"  Update PLUGIN_TOPICS and every profile that names them.",
                file=sys.stderr,
            )
            return 1

    problems = []
    checked = 0

    for path in profiles:
        app_id = path.stem
        if app_id not in apps:
            continue
        data = tomllib.loads(path.read_text(encoding="utf-8"))
        sub = data.get("event_bus", {}).get("subscribe")
        if sub is None:
            # No declaration: the app keeps its tier's exemption and the bus never
            # consults a list. Demanding one here would be an opinion about style.
            continue
        checked += 1
        missing = [t for t in PLUGIN_TOPICS if not covers(sub, t)]
        if missing:
            problems.append(
                f"{app_id}: declares a subscribe list and does not cover "
                f"{', '.join(missing)}.\n"
                f"    Its src-tauri links {PLUGIN}, which subscribes those from "
                f"inside the app's own process - so they arrive on this "
                f"connection under this profile. Declaring any list drops the "
                f"system-tier exemption, so the ones you did not name are "
                f"filtered out: the toolbar buttons and shortcuts stop arriving "
                f"and nothing reports it, because the subscribe itself succeeds."
            )

    if problems:
        print("an app that declares a subscribe list and loses its plugin's:")
        for p in problems:
            print(f"  {p}")
        return 1

    print(
        f"OK: {checked} declared subscribe list(s) across {len(apps)} plugin-linking "
        f"app(s), each covering both topics the plugin subscribes"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
