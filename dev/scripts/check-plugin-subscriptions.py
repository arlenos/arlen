#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Check that an app whose plugin subscribes for it is granted what the plugin asks.

WHY THIS EXISTS. `sdk/tauri-plugin-shell`'s `init` calls
`spawn_action_invoked_consumer` UNCONDITIONALLY, and that consumer subscribes to
`app.toolbar.action_invoked`, `app.shortcut.action_invoked` and
`app.menu.action_invoked` from inside the app's own process. So linking the plugin
IS subscribing to those three - there is no line in the app's source that says so,
and reading the app will never show it.

The menu topic joined the list on 9 September, and it is why the count went from
two to three: eleven apps were listening for `arlen://menu-action` and nothing
relayed it, so their top-bar menus were drawn and did nothing. The relay went into
the plugin, which made the grant a CONSEQUENCE of linking rather than a choice -
which is the only kind of thing this check may demand.

An app whose permission profile does not name them loses both quietly. The bus
filters the ungranted patterns out, `subscribe` still returns `Ok`, and the
plugin's self-healing loop waits on a receiver that never yields: the toolbar
buttons and the keyboard shortcuts stop working with nothing anywhere saying why.
The knowledge app's profile records that failure from an enforce boot on 14 August;
`dev.arlen.pdf` was still in it on 8 September, with no `[event_bus]` section at
all, which is what this check was written for.

WHAT IS CHECKED. Two halves, and each has its own airtight signal.

SUBSCRIBE: an app that DEPENDS on `tauri-plugin-arlen-shell` in its
`src-tauri/Cargo.toml` must have a shipped profile whose `[event_bus].subscribe`
contains all three patterns. Linking is subscribing, so the dependency is the
signal.

PUBLISH: an app whose FRONTEND calls `plugin:arlen-shell|menu_register` must be
granted `app.menu.registered`. This paragraph used to say the publish side could
not be checked - "an app only emits it if it drives that helper, and telling which
does is more than a grep can say" - and that was wrong twice over. The invoke IS
the app driving the helper, and it is as greppable as the dependency; and while
nobody checked, TWELVE of the thirteen profiles for apps that register a menu
granted no publish at all. Under enforcement every one of them registers no menu,
so the top-bar menu is absent for the whole desktop - the same defect the
subscribe list was widened to prevent on 9 September, one field over, found the
same evening in a bus log: `publish would be denied (shadow mode)
event_type=app.menu.registered`.

The reason it was missed is worth keeping, because it is the reading that keeps
producing this bug: every one of those profiles says some version of "nothing is
published: there is no emitter in this app", and every one of them is correct
about the app and wrong about the process. The emitter is in the plugin.

ONE PLUGIN, AND THAT IS THE WHOLE CLASS TODAY. Checked on 8 September: of the four
plugins in `sdk/`, only this one subscribes at all - `tauri-plugin-clipboard`,
`-menu` and `-portal` have no `.subscribe(` call site between them, and this one
has exactly one. So the rule below is not a sample of a bigger problem, it is the
problem. If a second plugin ever grows a consumer, this check is where its
patterns go, and the count in the passing line is what will look wrong first.

The plugin's OTHER background job, the theme watcher, needs no grant: it reads
through the plugin's own `theme_get` command rather than the bus.

Shown to fail before being trusted: `dev/scripts/test-check-plugin-subscriptions.mjs`.

Usage: check-plugin-subscriptions.py [repo-root]
"""

from __future__ import annotations

import re
import sys
import tomllib
from pathlib import Path

PLUGIN = "tauri-plugin-arlen-shell"
REQUIRED = (
    "app.toolbar.action_invoked",
    "app.shortcut.action_invoked",
    "app.menu.action_invoked",
)
PROFILE_DIR = "dev/mkosi/mkosi.extra/var/lib/arlen/permissions/1000"

# The frontend call that makes the plugin publish on the app's behalf, and what
# the app must then be granted. `app.menu.unregistered` is deliberately NOT
# demanded: nothing calls teardown today, so requiring it would be this check
# asking for a grant nobody uses.
MENU_REGISTER_INVOKE = "arlen-shell|menu_register"
MENU_PUBLISH = "app.menu.registered"

FRONTEND_SUFFIXES = (".ts", ".svelte")


def linking_apps(root: Path) -> list[tuple[str, Path]]:
    """Every app whose Tauri crate depends on the plugin, with its manifest."""
    out = []
    for manifest in sorted((root / "apps").glob("*/src-tauri/Cargo.toml")):
        if PLUGIN in manifest.read_text(errors="replace"):
            out.append((manifest.relative_to(root / "apps").parts[0], manifest))
    return out


def registers_a_menu(root: Path, app: str) -> bool:
    """Whether the app's frontend asks the plugin to publish its menu."""
    src = root / "apps" / app / "src"
    if not src.is_dir():
        return False
    for path in src.rglob("*"):
        if path.suffix not in FRONTEND_SUFFIXES:
            continue
        try:
            if MENU_REGISTER_INVOKE in path.read_text(errors="replace"):
                return True
        except OSError:
            continue
    return False


def profile_for(root: Path, app: str) -> Path | None:
    """The shipped profile for an app, under either spelling of its id."""
    for name in (f"dev.arlen.{app}.toml", f"{app}.toml"):
        path = root / PROFILE_DIR / name
        if path.is_file():
            return path
    return None


def covered(topic: str, granted: list[str]) -> bool:
    """Whether a granted list admits `topic`, by name or by a wildcard over it.

    The bus reads a trailing `.*` as a prefix on a dot boundary, so a profile
    granting `app.menu.*` has already granted `app.menu.action_invoked`. Demanding
    the literal beside the family it belongs to would be this check asking for a
    line that changes nothing, which is how a check teaches people to stop reading
    it.
    """
    for entry in granted:
        if not isinstance(entry, str):
            continue
        if entry == topic:
            return True
        if entry.endswith(".*") and topic.startswith(entry[:-1]):
            return True
    return False


def main() -> int:
    root = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
    apps = linking_apps(root)
    if not apps:
        print("check-plugin-subscriptions: no app links the plugin, so the scan is pointed wrong")
        return 1

    problems: list[str] = []
    for app, _ in apps:
        profile = profile_for(root, app)
        if profile is None:
            problems.append(f"  - {app}: links {PLUGIN} and has no shipped profile at all")
            continue
        try:
            doc = tomllib.loads(profile.read_text(errors="replace"))
        except tomllib.TOMLDecodeError as e:
            problems.append(f"  - {app}: {profile.name} does not parse ({e})")
            continue
        granted = doc.get("event_bus", {}).get("subscribe")
        if granted is None:
            problems.append(
                f"  - {app}: {profile.name} has no `[event_bus].subscribe`, so every pattern "
                "the plugin subscribes for it is ungranted"
            )
            continue
        missing = [p for p in REQUIRED if not covered(p, granted)]
        if missing:
            problems.append(f"  - {app}: {profile.name} does not grant {', '.join(missing)}")

        if registers_a_menu(root, app):
            published = doc.get("event_bus", {}).get("publish") or []
            if not covered(MENU_PUBLISH, published):
                problems.append(
                    f"  - {app}: {profile.name} registers a menu through the plugin and "
                    f"does not grant publishing {MENU_PUBLISH}"
                )

    if problems:
        print("Apps whose plugin subscribes for them without a grant:")
        print()
        print("\n".join(problems))
        print()
        print("  Linking the plugin IS subscribing to these, and calling menu_register IS")
        print("  publishing. Without the grants the bus filters them out while both calls")
        print("  still succeed: the toolbar and shortcuts go dead, and the menu never")
        print("  appears. Add them to `[event_bus]` in the app's profile.")
        return 1

    menus = sum(1 for app, _ in apps if registers_a_menu(root, app))
    print(
        f"check-plugin-subscriptions: {len(apps)} app(s) link the plugin, "
        f"each granted every pattern it subscribes on their behalf; "
        f"{menus} of them register a menu and may publish it"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
