#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a Tauri event a frontend listens for is one something sends.

A `listen("...")` with no sender is the quietest defect this tree produces. It
compiles, it type-checks, it runs, the handler is correct, and the event never
arrives - so the surface just never updates and nobody can tell that from a
surface with nothing to update.

Two shapes found the day this was written:

  * **A name with the wrong scheme.** The Quick Settings audio and bluetooth
    tiles listened for `arlen://audio-changed` and `arlen://bluetooth-changed`;
    the monitors in `audio.rs` and `bluetooth.rs` emit those names BARE. Both
    tiles refreshed on their five-second timer alone, while the indicators
    beside them - listening for the right name - updated at once. One surface
    slower than its neighbour, for a scheme prefix.

  * **A relay nobody built.** Thirteen apps listen for `arlen://menu-action`,
    the click coming back from their own top-bar menu. Two of them wrote a
    consumer for it; the other eleven did not, and the shared plugin does not
    forward that topic, so their menus are drawn and inert.

The rule is deliberately generous about what counts as a sender: the name has
to appear as a string literal ANYWHERE in the Rust the app links - its own
`src-tauri` and `core`, plus `sdk/`, since the shell plugin and ui-kit emit into
every app's webview. A name held in a `const` counts, a name in a doc comment
counts. What it cannot do is appear nowhere, which is the whole finding.

The reverse direction (an event emitted that nobody hears) is NOT checked here:
a shell emitting for a surface not yet built is ordinary, and gating it would
make the queue an argument rather than a list.
"""

import re
import sys
from pathlib import Path

# Events listened for with no sender, by app, with why. Every entry is a real
# defect rather than a tolerated shape.
CARRIED: dict[str, tuple[int, str]] = {
    "calendar": (1, "`arlen://menu-action`, the top-bar menu relay"),
    "clock": (1, "`arlen://menu-action`, the top-bar menu relay"),
    "knowledge": (1, "`arlen://menu-action`, the top-bar menu relay"),
    "mail": (1, "`arlen://menu-action`, the top-bar menu relay"),
    "meetings": (1, "`arlen://menu-action`, the top-bar menu relay"),
    "pdf": (1, "`arlen://menu-action`, the top-bar menu relay"),
    "screenshot": (1, "`arlen://menu-action`, the top-bar menu relay"),
    "store": (1, "`arlen://menu-action`, the top-bar menu relay; arlen-ui's app"),
    "system-monitor": (1, "`arlen://menu-action`, the top-bar menu relay"),
    "terminal": (1, "`arlen://menu-action`, the top-bar menu relay"),
    "text-editor": (1, "`arlen://menu-action`, the top-bar menu relay"),
    "viewers": (1, "`arlen://menu-action`, the top-bar menu relay"),
    "settings": (
        2,
        "`arlen://menu-action` (the top-bar menu relay) and "
        "`arlen://shell-config-changed`",
    ),
}

# The eleven-app half of the carried list is ONE cause: the app publishes its
# menu through `sdk/tauri-plugin-shell`, the shell pushes the click back onto the
# bus as `app.menu.action_invoked`, and nothing in those apps subscribes it. Two
# apps wrote their own consumer (`files`, `harness`); the rest listen for a relay
# that was never built. The fix belongs in the plugin, which already subscribes
# the toolbar and shortcut topics inside every app's process - and it crosses
# into another lane's app, so it is reported rather than taken.

LISTEN = re.compile(r'listen(?:<[^>]*>)?\(\s*"([a-z][a-z0-9:/._-]*)"')

FRONTEND_SUFFIXES = (".ts", ".svelte")
SKIP = ("/node_modules/", "/build/", "/.svelte-kit/", "/target/")


def rust_text(bases: list[Path]) -> str:
    """Every Rust source under `bases`, concatenated."""
    parts: list[str] = []
    for base in bases:
        if not base.is_dir():
            continue
        for path in base.rglob("*.rs"):
            if any(s in str(path) for s in SKIP):
                continue
            try:
                parts.append(path.read_text(errors="replace"))
            except OSError:
                continue
    return "\n".join(parts)


def listened(app_src: Path) -> set[str]:
    """Every Tauri event name this app's frontend listens for."""
    names: set[str] = set()
    for path in app_src.rglob("*"):
        if path.suffix not in FRONTEND_SUFFIXES or any(s in str(path) for s in SKIP):
            continue
        try:
            names |= set(LISTEN.findall(path.read_text(errors="replace")))
        except OSError:
            continue
    return names


def main() -> int:
    root = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
    apps_dir = root / "apps"
    if not apps_dir.is_dir():
        print("check-tauri-events-heard: no apps directory, so the scan is pointed wrong")
        return 1

    shared = rust_text([root / "sdk"])
    apps = 0
    problems: list[str] = []
    for app_dir in sorted(p for p in apps_dir.iterdir() if (p / "src").is_dir()):
        app = app_dir.name
        names = listened(app_dir / "src")
        if not names:
            continue
        apps += 1
        own = rust_text([app_dir / "src-tauri" / "src", app_dir / "core" / "src"])
        unsent = sorted(n for n in names if f'"{n}"' not in own and f'"{n}"' not in shared)
        allowed, why = CARRIED.get(app, (0, ""))
        if len(unsent) > allowed:
            problems.append(
                f"  - {app}: {len(unsent)} event(s) nothing sends, carried as {allowed}.\n"
                f"    {', '.join(unsent)}"
            )
        elif len(unsent) < allowed:
            problems.append(
                f"  - {app}: carried as {allowed} ({why}) and only {len(unsent)} left. "
                "Lower the number so a new one cannot hide behind it."
            )

    if apps == 0:
        print("check-tauri-events-heard: no frontend sources found, so the scan is pointed wrong")
        return 1

    if problems:
        print("Tauri events a frontend listens for and nothing sends:")
        print()
        print("\n".join(problems))
        print()
        print("  A listener with no sender is a surface that never updates, and it")
        print("  looks exactly like a surface with nothing to update.")
        return 1

    carried = sum(n for n, _ in CARRIED.values())
    print(f"check-tauri-events-heard: {apps} app(s) scanned, {carried} carried")
    return 0


if __name__ == "__main__":
    sys.exit(main())
