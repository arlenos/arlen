#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that systemd understands every directive in the shipped units.

A mistyped directive is not an error. systemd logs `Unknown key 'ProtectSytem'`
and carries on, so **the unit ships looking hardened and runs unhardened** - the
line that was supposed to confine a daemon holding an HMAC key or a token vault
does nothing, and the only evidence is one line in a journal nobody reads at boot.

Nothing else here catches that. `check-packaged-units.sh` compares the `dist/`
copy against the image copy directive-for-directive, which is the right check for
drift and passes happily when both copies carry the same typo. Reading the file
does not catch it either: `ProtectSytem` and `ProtectSystem` differ by one letter
in the middle of a word, which is precisely the shape human review skips.

So this asks systemd itself, via `systemd-analyze verify`, and fails on any
unknown key or invalid value.

**What is filtered, and why it is safe to filter:** the verifier also reports
that each `ExecStart=` binary is missing, because the binaries live in the image
and not on the host or a CI runner. That is expected everywhere this can run.
`check-shipped-units.py` is what holds ExecStart honest, against the crates that
actually build - a real check rather than an accident of where this runs.

Shown to fail before being trusted: `dev/scripts/test-check-unit-directives.mjs`
appends `ProtectSytem=strict` to a unit and expects a red.

Run: dev/scripts/check-unit-directives.py [tree]
"""

import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

# BOTH shipped trees. This gate was written on 11 Aug reading only the user tree,
# and a few hours later the peer-identity gate turned out to have the identical
# hole - the image ships five system units, and a mistyped directive in one of
# them would have been exactly as invisible as in a user unit. One tree is not a
# narrower check, it is a check with a blind side.
#
# Those two are all of them, checked rather than assumed: the only other
# `*.service` files in the image are one symlink under `etc/systemd/system`
# (`display-manager.service` -> the distro's `greetd.service`, the standard
# enablement alias, not a unit of ours) and two D-Bus activation files under
# `usr/share/dbus-1/services`, which share the extension and nothing else.
UNIT_DIRS = (
    ROOT / "dev/mkosi/mkosi.extra/usr/lib/systemd/user",
    ROOT / "dev/mkosi/mkosi.extra/usr/lib/systemd/system",
)

# The verdicts that mean a directive did not land. Everything else it says about
# a unit whose binary is not on this host is noise here.
BAD = re.compile(r"Unknown key|Invalid |Failed to parse|Unknown lvalue|Unknown section")


def main() -> int:
    if not any(d.is_dir() for d in UNIT_DIRS):
        print("no shipped unit tree, nothing to check")
        return 0
    if not shutil.which("systemd-analyze"):
        print("NOT CHECKED: systemd-analyze is not on this host, so no unit was verified")
        return 0

    units = sorted(u for d in UNIT_DIRS if d.is_dir() for u in d.glob("*.service"))
    problems = []
    for unit in units:
        r = subprocess.run(
            ["systemd-analyze", "verify", "--user", str(unit)],
            capture_output=True, text=True, check=False,
        )
        for line in f"{r.stdout}\n{r.stderr}".splitlines():
            if BAD.search(line):
                problems.append(f"{unit.name}: {line.strip()}")

    print(
        f"{len(units)} shipped unit(s) put through `systemd-analyze verify`: every "
        f"directive is one systemd knows. A key it does not know is dropped with a "
        f"log line and the unit runs without it, which is how a unit ships looking "
        f"hardened and running otherwise."
    )
    if problems:
        print("\nsystemd does not understand these, so they do nothing:\n")
        for p in problems:
            print(f"  - {p}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
