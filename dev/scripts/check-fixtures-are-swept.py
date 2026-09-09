#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that every host fixture is named by a sweep table.

A fixture in `dev/screenshot/hosts/` is the most expensive test artefact in this
tree: a `window.__TAURI_INTERNALS__` that answers the whole shape of every read,
refuses exactly one call, and drives the gesture that reaches a state no route
can. Writing one costs an afternoon. Running one costs nothing - IF something
runs it.

MEASURED, on 10 September: twenty-four fixtures existed and four were named by a
sweep. The other twenty ran only when somebody remembered `probe-host.sh`, and
that tool had been refusing every fixture for months because it compared a German
sentence against a filename. So the most careful work in the harness was also the
least exercised, and nothing said so - a fixture nobody runs looks exactly like a
fixture that passes.

WHAT COUNTS: an `@@name` in `sweep-render-all.sh`'s table. That is what makes the
fixture run on a German sweep of its app, with its declared `// EXPECT:` required
before the probes read the page.

WHAT IT CANNOT SEE: a row that names a fixture and a ROUTE the fixture cannot
reach its state on. That fails loudly the next time the sweep runs, which is the
difference between this check and the ones it is worth having.

Run: dev/scripts/check-fixtures-are-swept.py [root]
"""

import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
HOSTS = ROOT / "dev/screenshot/hosts"
TABLE = ROOT / "dev/screenshot/sweep-render-all.sh"

#: Fixture to why it is not in the table. Empty, and the bar for adding a name is
#: an argument that the state cannot be reached from a sweep row at all - not that
#: nobody has got to it yet.
CARRIED: dict[str, str] = {}

ROW = re.compile(r"@@([a-z0-9-]+)")


def main() -> int:
    if not HOSTS.is_dir():
        print(f"check-fixtures-are-swept: no {HOSTS}, so the scan is pointed wrong")
        return 1
    fixtures = sorted(HOSTS.glob("*.js"))
    if not fixtures:
        print("check-fixtures-are-swept: no host fixtures found, so the scan is pointed wrong")
        return 1
    if not TABLE.is_file():
        print(f"check-fixtures-are-swept: no table at {TABLE}, so the scan is pointed wrong")
        return 1

    named = set(ROW.findall(TABLE.read_text(encoding="utf-8", errors="replace")))
    if not named:
        print("check-fixtures-are-swept: the table names no fixture at all, so the scan is pointed wrong")
        return 1

    bad: list[str] = []
    carried = 0
    for f in fixtures:
        stem = f.stem
        if stem in named:
            continue
        if stem in CARRIED:
            carried += 1
            continue
        bad.append(
            f"  - {f.name} is named by no row in sweep-render-all.sh.\n"
            f"    Add `/route@@{stem}` to its app's row - or `/route::selector@@{stem}`\n"
            f"    when the state is behind a click, which `probe-host.sh` cannot do."
        )

    print(
        f"{len(fixtures)} host fixture(s); {len(named)} named by the sweep table,"
        f" {carried} carried with a reason."
        " Whether a named row actually REACHES its state is the sweep's own answer,"
        " every German run."
    )
    if bad:
        print("\nfixtures nothing runs:\n")
        print("\n".join(sorted(bad)))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
