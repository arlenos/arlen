#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Check that an app which claims to open files may read the file it is handed.

WHY. A desktop entry with `MimeType=` and `%f` in its `Exec=` is a promise to the file
manager: hand me one of these and I will open it. The permission profile is what a
confined launch actually allows. Nothing kept the two in agreement, and on 20 August
they disagreed: the calendar claimed `text/calendar` with `%f` while its profile
granted one directory - `$HOME/.local/share/arlen/calendars` - and nothing else. Under
a confined launch it would have refused the invitation the person had just
double-clicked, which is the app's headline way in, and the failure would have read as
a broken app rather than a missing grant.

It survived because the comment in that profile said the opposite ("no file picker and
no argument: it only ever looks in the directory it created itself"), which was untrue
when it was written - `agenda_of_file` had always read an arbitrary path - and the
existing profile check compares a description against its own GRANTS, so two statements
that agree with each other both pass while both are wrong about the code.

WHAT COUNTS. A user-directory grant: `home`, or `documents`/`downloads`/`pictures`/
`music`/`videos`. A `custom` list alone does NOT count, and that is the whole point -
the calendar had one, and it was the bug. A file arrives the ordinary way, as something
saved anywhere, so a grant naming one directory the app made itself cannot reach it.

This does not check the reverse. An app granted `home = true` that opens nothing is a
question about over-granting, which is a different check with a different argument.

Run: dev/scripts/check-openable-apps-can-read.py [root]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

#: Where the image's per-app profiles live, keyed by the app id the entry states.
PROFILES = ROOT / "dev/mkosi/mkosi.extra/var/lib/arlen/permissions/1000"

#: A grant that can reach a file the person saved somewhere. `custom` is absent on
#: purpose: see the module note.
USER_DIR_GRANTS = ("home", "documents", "downloads", "pictures", "music", "videos")

ARGV_FILE = re.compile(r"^Exec=.*%[fFuU]", re.M)
MIME = re.compile(r"^MimeType=\s*\S", re.M)
APP_ID = re.compile(r"^X-Arlen-AppId=(\S+)", re.M)


def grants_a_user_dir(profile: str) -> bool:
    """Whether the profile's `[filesystem]` section names a user directory."""
    section = profile.split("[filesystem]", 1)
    if len(section) < 2:
        return False
    # Up to the next section header, so a `documents = true` under `[network]`
    # (which would be a different bug) cannot satisfy this one.
    body = re.split(r"^\[", section[1], maxsplit=1, flags=re.M)[0]
    return any(re.search(rf"^\s*{g}\s*=\s*true", body, re.M) for g in USER_DIR_GRANTS)


def main() -> int:
    entries = sorted(ROOT.glob("apps/*/dist/*.desktop"))
    if not entries:
        print("NOTHING WAS READ: no desktop entries found, so this checked nothing", file=sys.stderr)
        return 2

    problems: list[str] = []
    checked = 0
    for entry in entries:
        text = entry.read_text(encoding="utf-8", errors="replace")
        if not (MIME.search(text) and ARGV_FILE.search(text)):
            continue
        checked += 1
        match = APP_ID.search(text)
        if not match:
            problems.append(
                f"{entry.relative_to(ROOT)} opens files but states no `X-Arlen-AppId`, "
                f"so no profile can be found for it"
            )
            continue
        profile = PROFILES / f"{match.group(1)}.toml"
        if not profile.is_file():
            # Not this check's business: an app can be unstaged and profileless,
            # and `check-app-profiles.py` is what says whether it may be.
            continue
        if not grants_a_user_dir(profile.read_text(encoding="utf-8", errors="replace")):
            problems.append(
                f"{entry.relative_to(ROOT)} claims a MimeType and takes a file on argv, but "
                f"{profile.relative_to(ROOT)} grants no user directory.\n"
                f"    A confined launch would refuse the file the person double-clicked, and the "
                f"failure reads as a broken app rather than a missing grant. Add `home = true` "
                f"(or the narrower user dir the app really needs) - a `custom` path naming a "
                f"directory the app made itself cannot reach a saved attachment."
            )

    if problems:
        print("apps that promise to open files they may not read:\n")
        for p in problems:
            print(f"  - {p}")
        return 1

    print(
        f"{checked} app(s) claim a file type and take a file on argv; each may read one. "
        f"Whether the grant is the NARROWEST that works is a question for a person."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
