#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A D-Bus-activated daemon must start through its unit, not as a bare Exec.

A `.service` file under `dbus-1/services` has two ways to say how to start a
daemon. `Exec=` runs the binary directly. `SystemdService=` hands the job to
systemd, which starts the unit - with `ProtectSystem=strict`, the `ReadWritePaths`
sandbox, the seccomp set and everything else the unit spent effort declaring.

Only the second is safe, and the difference is invisible: both start the daemon,
both work, and the bare-Exec one simply runs unconfined. `org.arlen.InstallDaemon1`
shipped exactly that way - alone among its siblings - so an install daemon
D-Bus-activated on a real system would have run outside the sandbox its own unit
defines, on the path that installs software as root. Found by reading the four
files side by side, not by anything failing.

Two refusals, because a pointer to nothing is as quiet as no pointer:

    1. Every `org.arlen.*.service` activation file names a `SystemdService=`.
    2. That unit exists in the tree. A typo'd or renamed unit leaves activation
       to fail with "unit not found" at the moment a user first needs the daemon,
       which is the worst time to discover a filename.

NOT covered: whether the unit is actually installed on an image (an activation
file for a daemon that does not ship yet is normal here - installd and the
online-accounts daemon are both in that state), and whether `Exec=` and the unit's
`ExecStart=` agree. The first is `check-shipped-units.py`; the second would be
worth having and needs the two paths reconciled first, since the activation file
names an install path and the unit may name a different one.

Shown to fail before being trusted: `dev/scripts/test-check-dbus-activation.mjs`
plants a missing pointer and a dangling one.
"""

import pathlib
import re
import sys

ROOT = (
    pathlib.Path(sys.argv[1]).resolve()
    if len(sys.argv) > 1
    else pathlib.Path(__file__).resolve().parents[2]
)
SKIP = ("target", "mkosi.builddir", "mkosi.cache", "node_modules")

# activation file stem -> why it needs no SystemdService.
UNPOINTED: dict[str, str] = {}


def activation_files() -> list[pathlib.Path]:
    """Every arlen D-Bus activation file in the tree, generated copies aside."""
    return sorted(
        p
        for p in ROOT.rglob("org.arlen.*.service")
        if not set(SKIP) & set(p.parts)
    )


def unit_exists(name: str) -> bool:
    """Whether a unit of that filename is anywhere in the tree."""
    return any(not set(SKIP) & set(p.parts) for p in ROOT.rglob(name))


def main() -> int:
    files = activation_files()
    if not files:
        print(
            "NOTHING WAS READ: no org.arlen.*.service activation files found",
            file=sys.stderr,
        )
        return 2

    problems: list[str] = []
    for path in files:
        rel = path.relative_to(ROOT)
        text = path.read_text(encoding="utf-8", errors="replace")
        stem = path.name
        m = re.search(r"^SystemdService=(\S+)", text, re.M)
        if not m:
            if stem in UNPOINTED:
                continue
            problems.append(
                f"{rel} has no SystemdService=, so D-Bus activation runs its "
                f"Exec= directly and the daemon starts OUTSIDE the sandbox its "
                f"unit declares. Both paths start it; only one is confined."
            )
            continue
        unit = m.group(1)
        if not unit_exists(unit):
            problems.append(
                f"{rel} points at {unit}, and no such unit is in the tree. "
                f"Activation then fails the first time a user needs the daemon."
            )

    for stale in sorted(UNPOINTED):
        if not any(p.name == stale for p in files):
            problems.append(f"{stale} is excused here and has no activation file; delete the entry")

    if problems:
        print("dbus activation:\n")
        for p in problems:
            print(f"  - {p}")
        return 1

    print(
        f"{len(files)} arlen D-Bus activation file(s); each starts its daemon "
        f"through a unit that exists ({len(UNPOINTED)} excused)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
