# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that every MIME type an app claims has a default handler on the image.

WHAT THIS IS FOR. The launch service picks a handler ONLY from `[Default
Applications]` in a `mimeapps.list`, with no fallback to `[Added Associations]` -
that is deliberate, because the association list answers "what could open this"
and picking from it silently is a launch nobody chose. So a `MimeType=` line in a
desktop entry is a claim, not an association: something has to turn it into a
default. Nothing did, and the image shipped no list at all, so every document type
five first-party apps declare answered NoHandler on a fresh install.

The list is now shipped at `usr/share/applications/mimeapps.list` under
`mkosi.extra`. This keeps it in step with the entries in both directions: a type
an entry declares must have a default, and a default must name an entry that is
actually on the image and actually claims that type. The second half is what stops
the file rotting into a set of associations to applications nobody ships.

There is no acknowledgement list. Every type here comes from an entry the tree
itself writes, so a missing default is a line somebody forgot rather than a
decision somebody made.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LIST = ROOT / "dev/mkosi/mkosi.extra/usr/share/applications/mimeapps.list"


def claimed(root: Path) -> dict[str, str]:
    """Every MIME type a shipped desktop entry declares, mapped to that entry."""
    out: dict[str, str] = {}
    for entry in sorted(root.glob("apps/*/dist/*.desktop")):
        text = entry.read_text(errors="replace")
        m = re.search(r"^MimeType=(.*)$", text, re.M)
        if not m:
            continue
        for t in (x.strip() for x in m.group(1).split(";")):
            if t:
                out[t] = entry.name
    return out


def installed_entry_names(root: Path) -> set[str]:
    """Every `.desktop` basename a build phase installs under `applications/`.

    A default names a DESKTOP ID, and the id is the file name as installed. The
    list here is derived from `apps/*/dist/*.desktop`, so an entry the image
    happens to install under a different name would leave every default for it
    pointing at a file the image does not have - and the launcher answers
    NoHandler for a type this file says is covered. Checked rather than assumed,
    because the two names agreeing today is a convention, not a rule.
    """
    out: set[str] = set()
    for phase in sorted((root / "dev/mkosi/mkosi.build.d").glob("*")):
        if not phase.is_file():
            continue
        for line in phase.read_text(errors="replace").splitlines():
            if line.lstrip().startswith("#"):
                continue
            for m in re.findall(r"applications/([A-Za-z0-9._-]+\.desktop)", line):
                out.add(m)
    return out


def defaults(path: Path) -> dict[str, str]:
    """The `[Default Applications]` group, as type to first desktop id."""
    out: dict[str, str] = {}
    group = False
    for line in path.read_text(errors="replace").splitlines():
        line = line.strip()
        if line.startswith("["):
            group = line == "[Default Applications]"
            continue
        if not group or not line or line.startswith("#"):
            continue
        if "=" in line:
            mime, ids = line.split("=", 1)
            first = next((i for i in ids.split(";") if i.strip()), "")
            out[mime.strip()] = first.strip()
    return out


def main() -> int:
    want = claimed(ROOT)
    if not want:
        print("NOTHING WAS READ: no desktop entry declares a MimeType", file=sys.stderr)
        return 2
    if not LIST.is_file():
        print(f"NOTHING WAS READ: no association list at {LIST}", file=sys.stderr)
        return 2
    have = defaults(LIST)
    if not have:
        print(f"NOTHING WAS READ: no [Default Applications] in {LIST}", file=sys.stderr)
        return 2

    findings = []
    for mime, entry in sorted(want.items()):
        if mime not in have:
            findings.append(
                f"{entry} claims `{mime}` and nothing defaults to it, so opening one "
                f"answers NoHandler"
            )
        elif have[mime] != entry:
            findings.append(
                f"`{mime}` defaults to {have[mime]}, but {entry} is the entry that "
                f"claims it"
            )
    for mime, entry in sorted(have.items()):
        if mime not in want:
            findings.append(
                f"`{mime}` defaults to {entry}, and no shipped entry claims that type"
            )
    installed = installed_entry_names(ROOT)
    if installed:
        for entry in sorted(set(have.values())):
            if entry not in installed:
                findings.append(
                    f"`{entry}` is named as a default and no build phase installs a "
                    f"file by that name, so the id points at nothing on the image"
                )

    if findings:
        print(f"{len(want)} claimed type(s), {len(findings)} finding(s):\n", file=sys.stderr)
        for f in findings:
            print(f"  - {f}", file=sys.stderr)
        print(
            "\nThe list is derived from the entries: regenerate it rather than "
            "editing one side.",
            file=sys.stderr,
        )
        return 1

    print(
        f"{len(want)} MIME type(s) claimed by a shipped app, every one with a default "
        f"naming the entry that claims it. A `MimeType=` line is a claim; only a "
        f"default opens anything."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
