#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Every shipped user unit has something that actually starts it.

WHY THIS EXISTS. System units and user units are started by two different
mechanisms, and only one of them reads `[Install]`:

    system unit   mkosi's preset pass acts on `[Install] WantedBy=` at build
                  time and writes the symlink into /etc
    user unit     the session supervisor calls StartUnit on the units in
                  `USER_UNIT_APP_IDS`, and on nothing else - unless the image
                  ships a `default.target.wants` symlink, which the user
                  manager reads directly

So a user unit carrying `WantedBy=default.target` and nothing else does not
start. Its `[Install]` looks like the enable and is inert.

The failure is silent in the worst way: the unit is in the image, its binary is
in the image, and the boot log says nothing at all - there is no line to grep
for, because nothing ever tried. `arlen-kg-probe` sat like this after moving
from the system manager to the user manager, and the verify verdict read the
absence as the probe failing to resolve an identity. `arlen-store-backend` sat
like this for longer, with its binary built and installed by phase 8k every
time.

WHAT COUNTS AS STARTED. Either route:

    in USER_UNIT_APP_IDS         the supervisor starts it (and names it)
    default.target.wants/<unit>  the user manager starts it

A unit that is deliberately not started - socket-activated, or pulled in by
another unit's `Wants=` - is named in NOT_STARTED_ON_PURPOSE with the reason,
so the difference between "decided" and "forgotten" stays visible.
"""

import re
import sys
from pathlib import Path

OWN_TREE = len(sys.argv) <= 1
REPO = Path(__file__).resolve().parents[2] if OWN_TREE else Path(sys.argv[1]).resolve()

USER_UNITS = REPO / "dev/mkosi/mkosi.extra/usr/lib/systemd/user"
WANTS = USER_UNITS / "default.target.wants"
RESOLVER = REPO / "sdk/permissions/src/unit_identity.rs"

# Units nothing starts on purpose, with the reason. Empty today: every shipped
# user unit is meant to run. An entry here is a claim that some other mechanism
# brings the unit up, and it should name that mechanism.
NOT_STARTED_ON_PURPOSE: dict[str, str] = {}


def supervisor_start_list(src: str) -> set[str]:
    """The units the session supervisor calls StartUnit on."""
    if "USER_UNIT_APP_IDS: &[(&str, &str)] = &[" not in src:
        return set()
    block = src.split("USER_UNIT_APP_IDS: &[(&str, &str)] = &[")[1].split("];")[0]
    return set(re.findall(r'\("([^"]+\.service)"', block))


def main() -> int:
    if not USER_UNITS.is_dir():
        print(f"NOTHING WAS READ: no user units under {USER_UNITS}", file=sys.stderr)
        return 2

    shipped = sorted(p.name for p in USER_UNITS.glob("*.service"))
    if not shipped:
        print(f"NOTHING WAS READ: no .service files in {USER_UNITS}", file=sys.stderr)
        return 2

    started_by_supervisor = (
        supervisor_start_list(RESOLVER.read_text(encoding="utf-8"))
        if RESOLVER.is_file()
        else set()
    )
    linked = {p.name for p in WANTS.glob("*.service")} if WANTS.is_dir() else set()

    stranded = [
        u
        for u in shipped
        if u not in started_by_supervisor
        and u not in linked
        and u not in NOT_STARTED_ON_PURPOSE
    ]

    if stranded:
        print("shipped user unit(s) that nothing starts:")
        for u in stranded:
            print(f"  {u}")
        print(
            "\n  `[Install] WantedBy=` does not enable a USER unit - mkosi's preset\n"
            "  pass only acts on system units. Either add the unit to\n"
            "  USER_UNIT_APP_IDS, so the session supervisor starts it and its peers\n"
            "  can name it, or ship\n"
            "  mkosi.extra/usr/lib/systemd/user/default.target.wants/<unit>.\n"
            "  Leaving it as-is means the unit never runs and the boot log is\n"
            "  silent about it."
        )
        return 1

    print(
        f"OK: {len(shipped)} user unit(s), each started by the supervisor "
        f"({len(started_by_supervisor & set(shipped))}) or a wants-link ({len(linked)})"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
