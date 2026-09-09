#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a fixture's `// EXPECT:` is a sentence something still says.

Every host fixture in `dev/screenshot/hosts/` declares the words its state puts on
screen, and `probe-host.sh` refuses to report anything unless the page actually
carries them. That makes the EXPECT load-bearing: when the copy is reworded, the
fixture stops passing and the reason is one word in a file nobody opens.

MEASURED, on 10 September: `knowledge-refuses-search-save` declared
`Nicht gespeichert` while the surface says `Diese Suche wurde nicht gespeichert.` -
the same words mid-sentence, lowercase n, so the substring check missed on the
case alone. It had been stale for as long as nobody could run the fixture, which
was months.

THE RULE: an EXPECT must be either a sentence in an app's message catalogue, or a
string the fixture itself supplies. The second half matters - a device name, a
media title, a hostname, a backend's own error text all come from the fixture's
mocked data, and those are as stable as the fixture is.

WHAT IT CANNOT SEE: a sentence that exists in the catalogue but is no longer
REACHED by the surface this fixture drives, and a formatted value (a number, a
date) whose rendered form differs from the data it came from. One of those is
carried below.

Run: dev/scripts/check-fixture-expect-still-said.py [root]
"""

import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
HOSTS = ROOT / "dev/screenshot/hosts"

#: Fixture to (its EXPECT, why it is in neither place). Only formatting: the
#: fixture supplies `15.8` and the page renders `15,8` in German, and no static
#: check can resolve a locale's number format back to its source.
CARRIED: dict[str, tuple[str, str]] = {
    "monitor-live-tick.js": ("15,8", "the fixture supplies 15.8; the surface formats it German"),
}

EXPECT = re.compile(r"^// EXPECT: *(.+)$", re.M)
CATALOG = re.compile(r'^\s*"[\w.$-]+"\s*:\s*"([^"]+)"', re.M)
ESCAPE = re.compile(r"\\u([0-9a-fA-F]{4})")


def unescaped(text: str) -> str:
    """Catalogue values carry `\\u00fc` for an umlaut, and an EXPECT is written as
    the letter. Comparing the two without this reported a sentence that is on
    screen in front of you as one nothing says."""
    return ESCAPE.sub(lambda m: chr(int(m.group(1), 16)), text)


def catalog_text(root: Path) -> str:
    out: list[str] = []
    for base in ("apps", "sdk"):
        d = root / base
        if not d.is_dir():
            continue
        for f in sorted(d.rglob("messages*.ts")):
            if "node_modules" in f.parts or ".svelte-kit" in f.parts:
                continue
            out += CATALOG.findall(f.read_text(encoding="utf-8", errors="replace"))
    return unescaped("\n".join(out))


def main() -> int:
    if not HOSTS.is_dir():
        print(f"check-fixture-expect-still-said: no {HOSTS}, so the scan is pointed wrong")
        return 1
    fixtures = sorted(HOSTS.glob("*.js"))
    if not fixtures:
        print("check-fixture-expect-still-said: no host fixtures found, so the scan is pointed wrong")
        return 1
    said = catalog_text(ROOT)
    if not said:
        print("check-fixture-expect-still-said: no catalogue read, so the scan is pointed wrong")
        return 1

    bad: list[str] = []
    carried = 0
    declared = 0
    for f in fixtures:
        src = f.read_text(encoding="utf-8", errors="replace")
        m = EXPECT.search(src)
        if not m:
            # `probe-host.sh` already refuses a fixture with no EXPECT, and says
            # so in a sentence written for whoever is holding the fixture. This
            # check is about drift, not about absence.
            continue
        declared += 1
        want = m.group(1).strip()
        if want in said:
            continue
        if want in src.replace(m.group(0), ""):
            continue
        entry = CARRIED.get(f.name)
        if entry and entry[0] == want:
            carried += 1
            continue
        bad.append(
            f"  - {f.name}: expects `{want}`, which no catalogue carries and the\n"
            f"    fixture does not supply. Either the copy moved and this is stale, or\n"
            f"    it is a formatted value and belongs in CARRIED with that reason."
        )

    print(
        f"{declared} fixture(s) declare what their state says; {carried} carried as formatting."
        " A sentence that still exists but is no longer REACHED by the surface reads"
        " clean here and is the driven check's job."
    )
    if bad:
        print("\nfixtures expecting a sentence nothing says:\n")
        print("\n".join(sorted(bad)))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
