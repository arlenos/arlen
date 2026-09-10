#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that nothing starts an Xvfb with a display number it does not own.

`xvfb-run -a` looks for a free display and THEN creates its lock, and those two
steps are not atomic. Two renders starting together take the same `:N`, the
loser's server goes away under openbox and WebKit, and both runs hang with no
output and no exit - a stuck display, which reads exactly like a broken probe and
is the most expensive false signal this harness produces.

WHY A CHECK RATHER THAN A FIXED SCRIPT. The fix is a per-pid display number and
it was written on 10 September - into `headless.sh`, which READS a page, and not
into `shoot.sh`, which takes the PICTURE. A sweep runs both on every row, so half
of every row went on racing while the harness read as fixed, and it surfaced a day
later as a sweep aborting at its control the moment a second render started. That
is the drift this check exists to stop: the loop now lives in one file and this
says so out loud whenever a new caller reaches for `xvfb-run` directly.

THE RULE: `xvfb-run` is invoked in `dev/screenshot/lib/own-display.sh` and nowhere
else. Everything else calls `own_display`. Naming the binary in a
`command -v` guard or a comment is not invoking it.

Run: dev/scripts/check-owned-display.py [root]
"""

import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
DEV = ROOT / "dev"
HELPER = "dev/screenshot/lib/own-display.sh"

#: A line that RUNS it, as opposed to one that mentions it. `command -v xvfb-run`
#: and `for bin in xvfb-run xdotool` are availability guards, and a script that
#: cannot find the binary saying so is the opposite of the problem here.
INVOKES = re.compile(r"(?:^|[|;&(]|\$\()\s*(?:[A-Z_]+=\S+\s+)*xvfb-run\b")
GUARD = re.compile(r"command -v\s+xvfb-run|for bin in .*xvfb-run|require_xvfb")


def offenders(root: Path) -> tuple[list[str], int]:
    bad: list[str] = []
    scanned = 0
    for path in sorted(root.rglob("*.sh")):
        if "node_modules" in path.parts:
            continue
        scanned += 1
        rel = str(path.relative_to(ROOT))
        for n, line in enumerate(path.read_text(encoding="utf-8", errors="replace").splitlines(), 1):
            stripped = line.strip()
            if stripped.startswith("#") or not INVOKES.search(line) or GUARD.search(line):
                continue
            if rel == HELPER:
                continue
            bad.append(
                f"  - {rel}:{n} runs xvfb-run itself. Two renders can then take the\n"
                f"    same display and both hang. Source {HELPER} and call\n"
                f"    `own_display \"<server args>\" <command...>` instead."
            )
    return bad, scanned


def main() -> int:
    if not DEV.is_dir():
        print(f"check-owned-display: no {DEV}, so the scan is pointed wrong")
        return 1
    if not (ROOT / HELPER).is_file():
        print(f"check-owned-display: no {HELPER}; the rule names a file that is not there")
        return 1
    bad, scanned = offenders(DEV)
    if not scanned:
        print("check-owned-display: no shell scripts under dev/, so the scan is pointed wrong")
        return 1
    print(
        f"{scanned} shell script(s) under dev/ read; every Xvfb is started through"
        f" {HELPER}, which picks a display number of its own rather than racing for one."
    )
    if bad:
        print("\nscripts that start an Xvfb on a display they do not own:\n")
        print("\n".join(bad))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
