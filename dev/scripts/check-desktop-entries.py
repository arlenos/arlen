#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Run `desktop-file-validate` over every entry the image installs, hints included.

WHY. A desktop entry is the only description of an app the launcher and the "open with"
list ever read, and it fails quietly: a bad `Categories` puts the app in the menu twice,
a bad `MimeType` makes it the default for files it cannot open, a missing `%f` makes a
file-opening app silently ignore the file it was handed. None of that shows up in a build
or a screenshot, and freedesktop ships a validator for exactly this.

HINTS ARE FAILURES HERE, not just errors. On 15 August the three entries written that
evening were all "valid" while each named more than one main category - the hint says an
app with two appears twice in the menu, and all three would have. Only the pre-existing
files entry got it right, which is the sort of drift that spreads by copying.

The validator is `desktop-file-utils`. When it is absent this check says so and passes:
the same convention the rest of the harness follows, since a missing tool must not block
a commit that has nothing to do with it.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

ROOT = (
    Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
)
APPS = ROOT / "apps"


def entries() -> list[Path]:
    """Every `dist/*.desktop` under apps/, which is where a staged entry lives."""
    return sorted(p for p in APPS.glob("*/dist/*.desktop") if p.is_file())


def main() -> int:
    if not APPS.is_dir():
        print("apps/ is missing; the layout moved and this check did not")
        return 1

    validator = shutil.which("desktop-file-validate")
    if not validator:
        print("desktop-file-validate not installed (desktop-file-utils); not run")
        return 0

    found = entries()
    if not found:
        print("no desktop entry found under apps/*/dist; that is not plausible")
        return 1

    problems: list[str] = []
    for entry in found:
        out = subprocess.run(
            [validator, str(entry)], capture_output=True, text=True, check=False
        )
        text = (out.stdout + out.stderr).strip()
        if out.returncode != 0 or text:
            problems.append(f"{entry.relative_to(ROOT)}:\n      " + text.replace("\n", "\n      "))

    if problems:
        print("\ndesktop entries the launcher would read wrongly:\n")
        for p in problems:
            print(f"  - {p}")
        print(
            "\nA hint counts. 'more than one main category' means the app appears twice in "
            "the menu, which is a real thing a user sees."
        )
        return 1

    print(f"{len(found)} desktop entrie(s) validate clean, hints included.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
