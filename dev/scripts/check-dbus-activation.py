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
    3. `Exec=` names the same binary as that unit's `ExecStart=`. systemd wins
       while `SystemdService=` is present, so a divergent `Exec=` is invisible
       until it is not: drop the pointer, or activate on a system where systemd
       is not the one answering, and D-Bus starts the OTHER path - typically an
       older install location that still exists. The two lines describing one
       daemon should not be able to disagree in silence.

I first recorded (3) as needing the two path conventions reconciled before it
could be checked. That was wrong and worth saying: all seven dist activation
files already name exactly what their unit names. Nothing needed reconciling, so
the check went in.

NOT covered: whether the unit is actually installed on an image. An activation
file for a daemon that does not ship yet is normal here - installd and the
online-accounts daemon are both in that state - and `check-shipped-units.py` is
where that question lives.

Shown to fail before being trusted: `dev/scripts/test-check-dbus-activation.mjs`
plants a missing pointer and a dangling one.
"""

import os
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


#: Every `.service` file in the tree, by file name, read once.
#:
#: `find_unit` used to `rglob` the whole repo PER activation file, and each of
#: those walks descended into every build directory before `SKIP` discarded what
#: it found. One pruned walk answers all of them.
_UNITS: dict[str, list[pathlib.Path]] | None = None


def units_by_name() -> dict[str, list[pathlib.Path]]:
    """Service files by file name, generated copies aside, sorted for stability."""
    global _UNITS
    if _UNITS is not None:
        return _UNITS
    found: dict[str, list[pathlib.Path]] = {}
    for base, dirs, files in os.walk(ROOT):
        dirs[:] = sorted(d for d in dirs if d not in SKIP)
        for name in sorted(files):
            if name.endswith(".service"):
                found.setdefault(name, []).append(pathlib.Path(base) / name)
    _UNITS = found
    return found


def activation_files() -> list[pathlib.Path]:
    """Every arlen D-Bus activation file in the tree, generated copies aside."""
    return sorted(
        p
        for name, paths in units_by_name().items()
        if name.startswith("org.arlen.")
        for p in paths
    )


def find_unit(name: str) -> pathlib.Path | None:
    """The first unit of that filename in the tree, generated copies aside."""
    return next(iter(units_by_name().get(name, [])), None)


def exec_binary(text: str, key: str) -> str | None:
    """The binary an `Exec=`/`ExecStart=` line runs, arguments and prefixes aside."""
    m = re.search(rf"^{key}=(\S+)", text, re.M)
    if not m:
        return None
    # systemd allows `+`, `!`, `-` and `@` prefixes on ExecStart; strip them so a
    # hardened unit is compared on the binary rather than on its decoration.
    return m.group(1).lstrip("+!-@")


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
        unit_path = find_unit(unit)
        if unit_path is None:
            problems.append(
                f"{rel} points at {unit}, and no such unit is in the tree. "
                f"Activation then fails the first time a user needs the daemon."
            )
            continue

        activation_exec = exec_binary(text, "Exec")
        unit_exec = exec_binary(
            unit_path.read_text(encoding="utf-8", errors="replace"), "ExecStart"
        )
        if activation_exec and unit_exec and activation_exec != unit_exec:
            problems.append(
                f"{rel} runs {activation_exec} while {unit} runs {unit_exec}. "
                f"systemd wins while SystemdService= is there, so the difference "
                f"is invisible until the pointer is dropped or systemd is not the "
                f"one answering - and then activation starts the other binary."
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
        f"through a unit that exists and runs the same binary "
        f"({len(UNPOINTED)} excused)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
