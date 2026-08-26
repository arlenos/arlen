#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""The viewer's desktop entry may claim exactly the formats its core can decode.

`check-desktop-entries` names this defect in its own opening - "a bad `MimeType`
makes it the default for files it cannot open" - and cannot check it:
`desktop-file-validate` knows the syntax of a MIME list and nothing about what
the app behind it can do. This is that half, for the one app where both sides are
machine-readable.

WHAT IS COMPARED. `apps/viewers/core` owns `IMAGE_MIMES` + `AUDIO_MIMES`, and
`apps/viewers/host/src/mimeapps.rs` builds the RUNTIME default-handler
registration from exactly those two. The desktop entry re-states the same set by
hand. So the app tells the world what it opens in two places, one derived and one
copied, and they must agree.

THEY DID NOT. On 26 August the entry carried `image/x-raw` and the const did not:
a RAW file resolves (`detect_by_extension` maps .cr2/.nef/.arw to
`Decoder::Fallback`) but `worker_bin(Fallback)` is `None` - "a later slice" - so
opening one answers "no image decoder for this format". The app registered itself
as handler for 22 types and advertised 23. The code already knew; the hand-copied
line had drifted.

WHY NOT WIDER. The other four entries that claim types (calendar, mail, pdf,
text-editor) have no machine-readable capability list to compare against - what a
text editor can open is not a const anywhere - so a check over them would be
asserting a list against itself. One app, honestly, beats five apps' worth of
theatre. If another app grows such a list, it joins the table below.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# app -> (desktop entry, rust source, const names). The one entry today; the
# shape is a table so a second app is data rather than a rewrite.
SUBJECTS: dict[str, tuple[str, str, tuple[str, ...]]] = {
    "viewers": (
        "apps/viewers/dist/arlen-viewers.desktop",
        "apps/viewers/core/src/lib.rs",
        ("IMAGE_MIMES", "AUDIO_MIMES"),
    ),
}

CONST = r'pub const {name}: &\[&str\] =\s*&\[(.*?)\];'
STRINGS = re.compile(r'"([^"]+)"')


def const_mimes(text: str, name: str) -> set[str] | None:
    """The string literals in `pub const <name>: &[&str] = &[...]`, or None."""
    m = re.search(CONST.format(name=re.escape(name)), text, re.S)
    if not m:
        return None
    return set(STRINGS.findall(m.group(1)))


def entry_mimes(text: str) -> set[str] | None:
    """The `MimeType=` set, or None when the entry declares none.

    Comment lines are skipped: this file's `MimeType` discussion NAMES the types
    it decided against (`image/*`, the video types, and now `image/x-raw` with the
    condition for putting it back), and reading those as claims would report the
    reasoning as the defect.
    """
    for line in text.splitlines():
        if line.startswith("#"):
            continue
        if line.startswith("MimeType="):
            return {t for t in line[len("MimeType="):].split(";") if t}
    return None


def main() -> int:
    root = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else ROOT
    findings: list[str] = []
    compared = 0

    for app, (entry_rel, src_rel, names) in sorted(SUBJECTS.items()):
        entry, src = root / entry_rel, root / src_rel
        if not entry.is_file() or not src.is_file():
            print(
                f"NOTHING WAS READ: {app} names {entry_rel} and {src_rel}, and one is "
                f"missing, so no claim was checked",
                file=sys.stderr,
            )
            return 2
        src_text = src.read_text(encoding="utf-8", errors="replace")

        declared: set[str] = set()
        for name in names:
            found = const_mimes(src_text, name)
            if found is None:
                print(
                    f"NOTHING WAS READ: {src_rel} no longer declares `{name}`, so the "
                    f"capability side of the comparison is gone. Re-point the table.",
                    file=sys.stderr,
                )
                return 2
            declared |= found

        claimed = entry_mimes(entry.read_text(encoding="utf-8", errors="replace"))
        if claimed is None:
            print(
                f"NOTHING WAS READ: {entry_rel} declares no MimeType, so there is no "
                f"claim to check",
                file=sys.stderr,
            )
            return 2

        compared += 1
        for t in sorted(claimed - declared):
            findings.append(
                f"{entry_rel}: claims `{t}`, which {src_rel} does not list as decodable. "
                f"The launcher offers this app for a file it answers with a refusal. "
                f"Drop it from the entry, or add it to the const once something decodes it."
            )
        for t in sorted(declared - claimed):
            findings.append(
                f"{entry_rel}: does NOT claim `{t}`, which {src_rel} lists as decodable "
                f"and the runtime handler registers. The app opens it and is never "
                f"offered for it."
            )

    print(
        f"{compared} desktop entr(ies) checked against the format set their app "
        f"actually decodes"
    )
    if findings:
        print("\na claim the app cannot keep, or a capability it never offers:\n")
        for f in findings:
            print(f"  - {f}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
