# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check the curated third-party profile catalogue for the drift a launch meets.

`sdk/permissions/profiles/` holds the authored profiles for third-party
applications - over two thousand of them, staged by
`mkosi.build.d/08p-profiles.sh.chroot` into `/usr/share/arlen/profiles`, where
the apt-enrolment hook matches one by package name when a package is installed.
`check-app-profiles.py` deliberately does NOT look at them: it compares the apps
the IMAGE installs against the profiles the image ships, and says so in its own
header. So the corpus has had nothing looking at it, and it is the half that
grows - a profile is added by hand every time a new application is covered.

Three rules, and each one is a way a profile fails at the moment somebody
launches the app rather than here:

  * it parses as TOML, and `[info] app_id` is the id its filename claims. The
    enrolment hook writes the file out under the id it was found by and the
    resolver then trusts what is inside it, so a disagreement enrols one app's
    grants under another's name.
  * `[info] tier` is present. The tier decides which resolver rule applies; a
    profile without one is a profile whose authority is whatever the parser
    defaulted to.
  * A WHOLE-HOME GRANT STATES ITS REASON. `filesystem.home = true` is the widest
    thing a profile here can say, and 158 of them say it - terminals, backup
    tools, disk-usage readers, archivers, all of which genuinely need it. The
    rule is not that the grant is wrong, it is that a grant this wide must carry
    the sentence explaining it, the way every other wide grant in this tree does.
    An unexplained one is indistinguishable from a copied template, and the next
    person cannot tell which of the two they are reading.

What this does NOT check: whether the grants match what the application actually
reads. That is a per-app judgement and each profile argues for its own in its
header.

Shown to fail before being trusted: the control hands it a fixture catalogue with
each fault in turn.
"""

import re
import sys
import tomllib
from pathlib import Path

# Takes the catalogue to scan as an argument so the check can be pointed at a
# fixture and SHOWN TO FAIL, the `check-app-profiles.py` convention.
ROOT = Path(__file__).resolve().parents[2]
CATALOGUE = (
    Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else ROOT / "sdk/permissions/profiles"
)

# A comment that is ABOUT the width of the grant, rather than any comment at all.
# Matching a bare `#` would pass the SPDX header and every unrelated note, which
# is the same as not checking.
REASON = re.compile(r"\bhome\b|whole|everything|anywhere|entire|all of", re.IGNORECASE)


def comments(text: str) -> str:
    """Every comment line in the file, joined."""
    return " ".join(
        line.strip().lstrip("#").strip()
        for line in text.splitlines()
        if line.strip().startswith("#")
    )


def main() -> int:
    if not CATALOGUE.is_dir():
        print(f"no profile catalogue at {CATALOGUE}; it moved and this check did not")
        return 1
    files = sorted(CATALOGUE.glob("*.toml"))
    if not files:
        print(f"{CATALOGUE} holds no profiles; it moved and this check did not")
        return 1

    problems: list[str] = []
    wide = 0
    for path in files:
        text = path.read_text(encoding="utf-8", errors="replace")
        try:
            declared = tomllib.loads(text)
        except tomllib.TOMLDecodeError as e:
            problems.append(f"{path.name} does not parse ({e}), so enrolling it refuses the app")
            continue
        info = declared.get("info")
        if not isinstance(info, dict):
            problems.append(f"{path.name} has no `[info]` table, so it names no app and no tier")
            continue
        if info.get("app_id") != path.stem:
            problems.append(
                f"{path.name} is named for {path.stem} and its `[info] app_id` says "
                f"{info.get('app_id')!r}. The hook finds the file by the id and then "
                f"trusts what is inside it, so these must agree."
            )
        if not info.get("tier"):
            problems.append(
                f"{path.name} states no `[info] tier`, so which resolver rule applies "
                f"to it is whatever the parser defaulted to."
            )
        if declared.get("filesystem", {}).get("home") is True:
            wide += 1
            if not REASON.search(comments(text)):
                problems.append(
                    f"{path.name} grants the whole home and no comment says why. The "
                    f"grant may well be right - a terminal or a backup tool needs it - "
                    f"but the widest thing this file can say has to carry the sentence "
                    f"that explains it, or nobody can tell it from a copied template."
                )

    print(
        f"{len(files)} curated profile(s): each parses, names its own app and states a "
        f"tier; {wide} grant the whole home and each says why."
    )
    if problems:
        print("\nprofiles that would not enrol as they read:\n")
        for p in problems:
            print(f"  - {p}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
