#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Check that every external tool the tree spawns has been classified.

WHY. `check-spawned-binaries.py` covers our OWN binaries and deliberately stops
there, on the reasoning that which third-party packages an image installs is a build
decision a script should not hold an opinion about. That reasoning is right and it
left a silence: nobody had measured how many of those tools the image actually has.

Measured 20 August, by mounting the built image and looking: of 38 external binaries
the tree spawns, **23 are not on it**. Not a rounding error - `nmcli` (the whole
network popover), `pactl` and `wpctl` (audio), `wl-copy`/`wl-paste` (clipboard),
`rfkill` (airplane mode), `powerprofilesctl` (power profiles), `xdg-mime` and
`gtk-launch` (open-with and launching), `flatpak` (every Flatpak path), `git`
(forage's own `install git+URL`). Each is built, driven on a developer host where the
tool exists, and inert on the machine we ship.

So this check does NOT say which packages to install. It says every spawned tool must
be CLASSIFIED - shipped, or absent with the surface that stops working. A new
`Command::new("something")` for a tool nobody has thought about fails here, and the
absent list is a work item rather than a silence.

Re-measure with:

    guestfish --ro -a dev/mkosi/arlen.raw run : mount-ro /dev/sda2 / : sh '...'

Run: dev/scripts/check-spawned-tools-classified.py [root]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

TREES = ["apps", "daemons", "sdk", "ai", "forage", "store-backend"]
SPAWN = re.compile(r'Command::new\("([a-z][a-z0-9_.-]*)"')

#: On the image, measured 20 August. Base system and things a package brought in.
SHIPPED = {
    "sh", "sleep", "uname", "systemctl", "systemd-cat", "systemd-inhibit", "journalctl",
    "lsblk", "findmnt", "fusermount", "dpkg", "bwrap", "fc-list", "xdg-user-dirs-update",
    "arlen-run",
    # From the `appstream` package, which mkosi.conf installs for the store's
    # catalogue; the forage pipeline runs `appstreamcli compose` to give a built
    # package its own catalogue and icon cache.
    "appstreamcli",
}

#: NOT on the image, measured the same way, each with what stops working. The value
#: is the consequence rather than an excuse: this list is a work item for whoever
#: decides what the distribution contains, and a reason like "not needed yet" would
#: hide that a shipped surface calls it.
ABSENT: dict[str, str] = {
    "nmcli": "the shell's whole network popover - wifi list, connect, VPN, connection details",
    "pactl": "audio: volume, mute, per-app streams",
    "wpctl": "audio again, the pipewire half",
    "wl-copy": "putting anything on the clipboard",
    "wl-paste": "reading the clipboard, and the clipboard history the KG records",
    "rfkill": "airplane mode, in the quick settings and the network popover",
    "powerprofilesctl": "the battery popover's power profiles",
    "udisksctl": "mounting a removable disk from the file manager",
    "xdg-mime": "asking or setting which app opens a file type",
    "gtk-launch": "launching a desktop entry by id",
    "update-desktop-database": "refreshing the MIME cache after an install, so a new app is openable",
    "flatpak": "every Flatpak path: install, uninstall, list. Blocks the Flathub catalogue too, since a card would offer an install button with nothing behind it",
    "git": "forage's `install git+URL` and its recipe-repo clone",
    "snapper": "the filesystem snapshot seam the undo model captures against",
    "wf-recorder": "screen recording from the quick settings",
    "ffmpeg": "the meeting recorder's encode step",
    "rclone": "the backup/sync destination",
    "zenity": "the GTK fallback file/dialog picker",
    "kdialog": "the KDE fallback picker, tried before zenity",
    "man": "the terminal's `#` man-page lookup",
    "glib-compile-schemas": "compiling a GSettings schema an installed app ships",
    "arlen-settings": "four call sites open Settings; it has no image build step",
    "arlen-harness": "the terminal's share-a-block entry launches it; no image build step",
}


def spawns() -> dict[str, list[str]]:
    """Every external tool the tree spawns, with one file that spawns it."""
    found: dict[str, list[str]] = {}
    for tree in TREES:
        base = ROOT / tree
        if not base.is_dir():
            continue
        for path in sorted(base.rglob("*.rs")):
            if "target" in path.parts or "node_modules" in path.parts:
                continue
            text = path.read_text(encoding="utf-8", errors="replace")
            for tool in SPAWN.findall(text):
                found.setdefault(tool, []).append(str(path.relative_to(ROOT)))
    return found


def main() -> int:
    found = spawns()
    if not found:
        print("NOTHING WAS READ: no spawn sites found, so this checked nothing", file=sys.stderr)
        return 2

    unclassified = {t: f for t, f in found.items() if t not in SHIPPED and t not in ABSENT}
    # STALENESS IS ONLY KNOWABLE OVER THE WHOLE TREE. On an arbitrary root - a
    # control's fixture, a subdirectory - "this entry is dead" and "this fixture
    # is small" look identical, and failing on the second would make the check
    # impossible to write a control for. Over the real repository the lists are
    # meant to shrink, and an entry nobody spawns any more is one to delete.
    own_repo = ROOT == Path(__file__).resolve().parents[2]
    stale = sorted((SHIPPED | ABSENT.keys()) - found.keys()) if own_repo else []

    problems = []
    for tool, files in sorted(unclassified.items()):
        problems.append(
            f"`{tool}` is spawned by {files[0]} and is in neither list.\n"
            f"    Classify it: on the image (add to SHIPPED, after checking) or not (add to "
            f"ABSENT with the surface that stops working). An unclassified spawn is a feature "
            f"that may already be inert on the machine we ship."
        )
    if stale:
        problems.append(
            f"classified but no longer spawned anywhere: {', '.join(stale)}.\n"
            f"    Delete the entries - the lists are meant to track the tree, not outlive it."
        )

    if problems:
        print("spawned tools nobody has classified:\n")
        for p in problems:
            print(f"  - {p}")
        return 1

    print(
        f"{len(found)} external tool(s) spawned: {len(found) - len(ABSENT & found.keys())} on the image, "
        f"{len(ABSENT.keys() & found.keys())} absent with the surface each one costs. "
        f"Whether the absent ones SHOULD ship is a distribution decision this does not make."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
