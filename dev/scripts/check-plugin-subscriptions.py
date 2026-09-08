#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Check that an app whose plugin subscribes for it is granted what the plugin asks.

WHY THIS EXISTS. `sdk/tauri-plugin-shell`'s `init` calls
`spawn_action_invoked_consumer` UNCONDITIONALLY, and that consumer subscribes to
`app.toolbar.action_invoked` and `app.shortcut.action_invoked` from inside the
app's own process. So linking the plugin IS subscribing to those two - there is no
line in the app's source that says so, and reading the app will never show it.

An app whose permission profile does not name them loses both quietly. The bus
filters the ungranted patterns out, `subscribe` still returns `Ok`, and the
plugin's self-healing loop waits on a receiver that never yields: the toolbar
buttons and the keyboard shortcuts stop working with nothing anywhere saying why.
The knowledge app's profile records that failure from an enforce boot on 14 August;
`dev.arlen.pdf` was still in it on 8 September, with no `[event_bus]` section at
all, which is what this check was written for.

WHAT IS CHECKED, and it is deliberately the airtight half: an app that DEPENDS on
`tauri-plugin-arlen-shell` in its `src-tauri/Cargo.toml` must have a shipped
profile whose `[event_bus].subscribe` contains both patterns. The plugin's other
side-effect - `app.menu.registered`, emitted through the `Menu` helper the plugin
manages - is NOT checked, because an app only emits it if it drives that helper,
and telling which does is more than a grep can say. A rule that guessed there
would produce false demands for a grant an app does not use, and the fastest way
to get a check ignored is to have it ask for grants nobody needs.

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
REQUIRED = ("app.toolbar.action_invoked", "app.shortcut.action_invoked")
PROFILE_DIR = "dev/mkosi/mkosi.extra/var/lib/arlen/permissions/1000"


def linking_apps(root: Path) -> list[tuple[str, Path]]:
    """Every app whose Tauri crate depends on the plugin, with its manifest."""
    out = []
    for manifest in sorted((root / "apps").glob("*/src-tauri/Cargo.toml")):
        if PLUGIN in manifest.read_text(errors="replace"):
            out.append((manifest.relative_to(root / "apps").parts[0], manifest))
    return out


def profile_for(root: Path, app: str) -> Path | None:
    """The shipped profile for an app, under either spelling of its id."""
    for name in (f"dev.arlen.{app}.toml", f"{app}.toml"):
        path = root / PROFILE_DIR / name
        if path.is_file():
            return path
    return None


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
                f"  - {app}: {profile.name} has no `[event_bus].subscribe`, so both patterns "
                "the plugin subscribes for it are ungranted"
            )
            continue
        missing = [p for p in REQUIRED if p not in granted]
        if missing:
            problems.append(f"  - {app}: {profile.name} does not grant {', '.join(missing)}")

    if problems:
        print("Apps whose plugin subscribes for them without a grant:")
        print()
        print("\n".join(problems))
        print()
        print("  Linking the plugin IS subscribing to these. Without the grant the bus filters")
        print("  them out, the subscribe still succeeds, and the toolbar and shortcuts go dead")
        print("  silently. Add them to `[event_bus].subscribe` in the app's profile.")
        return 1

    print(
        f"check-plugin-subscriptions: {len(apps)} app(s) link the plugin, "
        "each granted both patterns it subscribes on their behalf"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
