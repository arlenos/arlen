#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""A message key the markup asks for has to exist in the catalogue.

`$t("g.a11y.readerHint")` against a catalogue that does not hold that key renders
the key itself - a person sees `g.a11y.readerHint` where a sentence belongs. It
compiles, it typechecks, and svelte-check has nothing to say about it, because
the key is just a string.

ONE DIRECTION ONLY, and the asymmetry is deliberate rather than lazy.

This reports a key the code ASKS FOR and the catalogue does not have. It does NOT
report a key the catalogue has and nothing seems to use, and that restraint is
the whole reason it can be trusted: 47 files across the tree build keys at
runtime - `$t(error)`, `$t(tb.id)` - so a scan for literal keys cannot know which
entries those reach. Tree-wide the catalogues hold about nine hundred keys no
literal use names, and almost certainly most of them are reached that way.
Reporting them would invite deleting a string somebody's error path needs, which
is the failure this file exists to prevent, arriving through the file itself.

The sibling `check-read-grants-cover-queries.py` chose the same asymmetry for the
same reason, and its author nearly deleted a needed grant on the strength of a
regex that had not seen a multi-line query. Missing-and-asked-for has no such
ambiguity: the key is right there in the source, and it is absent.

BOTH LOCALES ARE CHECKED, since a key present in English and missing in German is
a sentence that vanishes for half the users rather than for none.
"""

import re
import sys
from pathlib import Path

OWN_TREE = len(sys.argv) <= 1
REPO = Path(__file__).resolve().parents[2] if OWN_TREE else Path(sys.argv[1]).resolve()

# A locale block inside a messages catalogue: `en: { "a.b": "…", … }`.
BLOCK = re.compile(r"\b([a-z]{2})\s*:\s*\{(.*?)\n\s*\}", re.S)
KEY = re.compile(r'"([a-zA-Z0-9_.\-]+)"\s*:')

# A literal key handed to the translator. A dynamic one - `$t(error)` - is
# invisible here on purpose; see the module docstring.
USE = re.compile(r'\$?\bt\(\s*"([a-zA-Z0-9_.\-]+)"')

SKIP = ("node_modules", "/.svelte-kit/", "/build/")


def catalogues(app_src: Path) -> dict[str, set[str]]:
    """Locale -> the keys it defines."""
    out: dict[str, set[str]] = {}
    for f in (app_src / "lib" / "i18n").glob("messages*.ts"):
        for m in BLOCK.finditer(f.read_text(encoding="utf-8", errors="replace")):
            out.setdefault(m.group(1), set()).update(KEY.findall(m.group(2)))
    return out


def uses(app_src: Path) -> dict[str, str]:
    """Literal key -> the first file that asks for it."""
    found: dict[str, str] = {}
    for f in list(app_src.rglob("*.svelte")) + list(app_src.rglob("*.ts")):
        if "/i18n/" in str(f) or any(s in str(f) for s in SKIP):
            continue
        for k in USE.findall(f.read_text(encoding="utf-8", errors="replace")):
            found.setdefault(k, str(f.relative_to(app_src.parent)))
    return found


def main() -> int:
    apps = sorted(p for p in (REPO / "apps").glob("*/src/lib/i18n") if p.is_dir())
    if not apps:
        print(f"NOTHING WAS READ: no app catalogue under {REPO}/apps", file=sys.stderr)
        return 2

    problems: list[str] = []
    seen_uses = 0

    for i18n in apps:
        src = i18n.parent.parent
        app = src.parent.name
        locales = catalogues(src)
        if not locales:
            problems.append(f"{app}: has an i18n directory and no catalogue this can read.")
            continue
        used = uses(src)
        seen_uses += len(used)
        for key, where in sorted(used.items()):
            absent = sorted(loc for loc, keys in locales.items() if key not in keys)
            if absent:
                problems.append(
                    f"{app}: `{key}` is asked for in {where} and missing from "
                    f"{', '.join(absent)}.\n"
                    f"    A missing key renders as the key, so a person reads "
                    f"`{key}` where a sentence belongs - in {', '.join(absent)} only, "
                    f"which is why this is easy to ship without seeing it."
                )

    if not seen_uses:
        print("NOTHING WAS READ: no literal message key was found in any app", file=sys.stderr)
        return 2

    if problems:
        print("a message key the catalogue does not have:")
        for p in problems:
            print(f"  {p}")
        return 1

    print(f"OK: {seen_uses} literal key use(s) across {len(apps)} app(s), every one defined in every locale")
    return 0


if __name__ == "__main__":
    sys.exit(main())
