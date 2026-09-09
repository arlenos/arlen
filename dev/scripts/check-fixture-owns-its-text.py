#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a host fixture does not press a button by a word the catalogue owns.

A fixture in `dev/screenshot/hosts/` drives a real gesture: it finds a control and
clicks it. When it finds that control by the words printed on it, the fixture is
keyed on a string somebody else maintains - and the day the copy improves, the
fixture stops reaching its state and says nothing, because the picture it produces
is of a page where the gesture never happened.

MEASURED, on 10 September: five fixtures were looking for a button reading
`Entfernen` for a confirm whose label had become `Widerrufen`. All five had been
pressing the opener and then waiting for a button that could never appear. Nobody
noticed because `probe-host.sh` was refusing every fixture in the tree for an
unrelated reason, so the whole class was invisible twice over.

THE RULE IS ABOUT OWNERSHIP, not about text. A fixture may match a string it typed
itself - `waypointer-refuses-run` types `nosuchcommand` and then looks for it, and
that is stable forever because the fixture owns both ends. It may not match a
string that appears in an app's message catalogue, because that string belongs to
whoever writes the copy and will move without anyone thinking about fixtures.

WHAT IT CANNOT SEE: a fixture matching a word that is not in a catalogue today but
becomes one later, and a control found by a class that gets renamed - the same
fragility one layer over, which no static check reaches.

Run: dev/scripts/check-fixture-owns-its-text.py [root]
"""

import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
HOSTS = ROOT / "dev/screenshot/hosts"

#: Fixture to (the words it may still match, why). The uninstall opener is the
#: one exception: that page has exactly one button carrying the label and nothing
#: structural to tell it from the rest, so closing it needs an `id` on a surface
#: another lane is working in. Each of the four says so at its own call site.
CARRIED: dict[str, tuple[str, str]] = {
    "settings-uninstall-refused.js": ("Deinstallieren", "the only button with that label on the page; wants an id"),
    "settings-uninstall-failed.js": ("Deinstallieren", "same page, same button"),
    "settings-uninstall-unknown.js": ("Deinstallieren", "same page, same button"),
    "settings-uninstall-unavailable.js": ("Deinstallieren", "same page, same button"),
}

#: A string the fixture compares against something on the page.
COMPARED = re.compile(r'(?:trim\(\)\s*===\s*|byText\(|indexOf\()\s*"([^"]{2,})"')

#: A string the fixture writes into the page itself. What it typed, it may match.
WRITTEN = re.compile(r'(?:setter\.call\([^,]+,\s*|\.value\s*=\s*)"([^"]{2,})"')

#: A catalogue entry: `"some.key": "the words",`
CATALOG = re.compile(r'^\s*"[\w.$-]+"\s*:\s*"([^"]+)"', re.M)


def catalog_words(root: Path) -> set[str]:
    """Every sentence any app or the kit prints, as a set."""
    out: set[str] = set()
    for base in ("apps", "sdk"):
        d = root / base
        if not d.is_dir():
            continue
        for f in sorted(d.rglob("messages*.ts")):
            if "node_modules" in f.parts or ".svelte-kit" in f.parts:
                continue
            out |= set(CATALOG.findall(f.read_text(encoding="utf-8", errors="replace")))
    return out


def code_only(text: str) -> str:
    """The fixture with its `//` comments blanked out.

    Not cosmetic: the fix for this very defect class is a comment naming the word
    that used to be pressed, and matching inside comments reported three fixtures
    for the sentence explaining why they no longer press it. A check that reads
    its own repair as the fault is worse than no check.
    """
    out = []
    for line in text.splitlines():
        i = line.find("//")
        out.append(line if i < 0 else line[:i])
    return "\n".join(out)


def main() -> int:
    if not HOSTS.is_dir():
        print(f"check-fixture-owns-its-text: no {HOSTS}, so the scan is pointed wrong")
        return 1
    fixtures = sorted(HOSTS.glob("*.js"))
    if not fixtures:
        print("check-fixture-owns-its-text: no host fixtures found, so the scan is pointed wrong")
        return 1
    words = catalog_words(ROOT)
    if not words:
        print("check-fixture-owns-its-text: no catalogue read, so the scan is pointed wrong")
        return 1

    bad: list[str] = []
    carried = 0
    checked = 0
    for f in fixtures:
        text = code_only(f.read_text(encoding="utf-8", errors="replace"))
        own = set(WRITTEN.findall(text))
        for hit in set(COMPARED.findall(text)):
            if hit in own or hit not in words:
                continue
            checked += 1
            allowed = CARRIED.get(f.name)
            if allowed and allowed[0] == hit:
                carried += 1
                continue
            bad.append(
                f"  - {f.name}: presses `{hit}`, which is a sentence the catalogue owns.\n"
                f"    Find the control by its structure instead - a class, a container, its\n"
                f"    position in a dialog - or match a string this fixture typed itself."
            )

    print(
        f"{len(fixtures)} host fixture(s) checked against {len(words)} catalogue sentence(s);"
        f" {checked} text match(es) on copy, {carried} carried with a reason."
        " A control found by a class that later gets renamed is the same fragility"
        " and is not visible here."
    )
    if bad:
        print("\nfixtures pressing a word somebody else maintains:\n")
        print("\n".join(sorted(bad)))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
