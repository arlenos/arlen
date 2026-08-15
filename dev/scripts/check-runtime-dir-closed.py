#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""`/run/arlen` is not user-writable, which is what authenticates the broker.

The undo signer connects to the identity broker inside a mount namespace, where
`SO_PEERCRED` reports a root-owned peer as `nobody` and no uid check can work. It
does not need one: `/run/arlen` is created root-owned and non-user-writable, so a
node inside it was bound by root and the connect is authenticated by the filesystem
before any uid is consulted. See `lookup_identity_authenticated`.

That makes the directory's mode a load-bearing security property held in a systemd
unit, and the day it changes there is no other symptom - the signer keeps working,
it just stops being sure who it is talking to. So it gets a gate, exactly like the
inert-switch and socket-table facts.

Four things have to hold, and the middle two are the ones a well-meaning edit breaks:

    declared   some shipped unit creates `arlen` as a RuntimeDirectory
    root       that unit runs as root, so the directory is root-owned
    mode       its RuntimeDirectoryMode grants no write to group or other
    single     no OTHER shipped unit declares the same directory with a wider mode

The last one matters because `RuntimeDirectory=` is per-unit: a second unit naming
`arlen` with a laxer mode would quietly widen it depending on start order.
"""

import re
import sys
from pathlib import Path

REPO = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

# Only the SYSTEM tree. A user unit's `RuntimeDirectory=` lands under
# `/run/user/<uid>/`, which is the user's own directory by definition and can never
# provide this guarantee - so a user unit naming `arlen` is not the thing being
# checked here, and treating it as one would report a false match.
UNIT_DIR = REPO / "dev/mkosi/mkosi.extra/usr/lib/systemd/system"

# The directory whose mode is the credential.
GUARDED = "arlen"


def units_declaring(name):
    """(unit, mode, user) for every system unit creating `name` as a RuntimeDirectory."""
    out = []
    if not UNIT_DIR.is_dir():
        return out
    for unit in sorted(UNIT_DIR.glob("*.service")):
        text = unit.read_text(encoding="utf-8")
        # `RuntimeDirectory=` takes a space-separated list, so match the whole
        # value and look for the name among its words rather than anchoring on it.
        declares = any(
            name in value.split()
            for value in re.findall(r"^RuntimeDirectory=(.*)$", text, re.M)
        )
        if not declares:
            continue
        # The LAST occurrence of each, because that is the one systemd honours.
        # Reading the first hid a real case: the config broker already carries
        # `User=root` near the top, so a second `User=` further down - which is how
        # an override or a careless edit would arrive - was invisible here while
        # being the value that actually took effect.
        modes = re.findall(r"^RuntimeDirectoryMode=(\S+)", text, re.M)
        users = re.findall(r"^User=(\S+)", text, re.M)
        out.append((unit.name, modes[-1] if modes else None, users[-1] if users else "root"))
    return out


def mode_is_closed(mode):
    """Whether an octal RuntimeDirectoryMode denies write to group and other.

    systemd's default is 0755 and it is written four digits or three; anything we
    cannot parse is refused rather than assumed safe, because a mode nobody can
    read is exactly the case where a reader stops checking.
    """
    if mode is None:
        return True, "unset, so systemd's default 0755 applies"
    try:
        bits = int(mode, 8)
    except ValueError:
        return False, f"{mode!r} is not an octal mode"
    if bits & 0o022:
        return False, f"{mode} grants write to group or other"
    return True, f"{mode} denies write to group and other"


def main():
    if not UNIT_DIR.is_dir():
        print(f"NOTHING WAS READ: {UNIT_DIR} is not a directory", file=sys.stderr)
        return 2

    found = units_declaring(GUARDED)
    problems = []

    if not found:
        problems.append(
            f"no shipped system unit creates /run/{GUARDED} as a RuntimeDirectory.\n"
            f"    The identity broker's callers authenticate it by that path being\n"
            f"    root-owned and non-user-writable. With nothing declaring it, the\n"
            f"    directory's mode is whatever created it, and the check rests on air."
        )

    for unit, mode, user in found:
        ok, why = mode_is_closed(mode)
        if not ok:
            problems.append(
                f"{unit}: RuntimeDirectoryMode {why}.\n"
                f"    A user who can write /run/{GUARDED} can bind a socket there and\n"
                f"    be trusted as the broker. Keep it 0755 or tighter."
            )
        if user != "root":
            problems.append(
                f"{unit}: User={user}, so /run/{GUARDED} is owned by that account.\n"
                f"    That is fine only if it is a service account no login user can\n"
                f"    become. If it is the desktop user, the path stops bounding who\n"
                f"    bound the socket."
            )

    if problems:
        print(f"/run/{GUARDED} may not be closed to the user:")
        for p in problems:
            print(f"  {p}")
        return 1

    for unit, mode, user in found:
        _, why = mode_is_closed(mode)
        print(f"OK: /run/{GUARDED} from {unit} as {user}, mode {why}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
