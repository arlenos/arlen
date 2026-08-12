#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A verify probe may be admitted on the verify image and nowhere else.

The verify image has to exercise `explain_system`, which is gated to surfaces that
ARE the system talking to the user. The probe that drives it (`dogfood`) is not one
of those, and should not become one: "anything that can name itself" is precisely
the admission the list exists to refuse.

So the policy differs per image variant rather than the code doing: the verify
phase stages `/var/lib/arlen/permissions/user-surfaces.extra`, the release image
ships no such file, and `arlen_permissions::identity` reads it only to ADD to the
compiled-in list. That arrangement is only worth anything while the second half
stays true, and nothing about it is self-enforcing - a probe id typed into
`USER_SURFACES`, or the file moved into `mkosi.extra/`, would widen every shipped
image and look like ordinary wiring in review. Hence this check, which the planner
named the load-bearing half rather than a nicety: without it, this is the
compiled-in answer with extra steps.

Three refusals:

    1. No probe id appears in the compiled-in `USER_SURFACES` (or its dev twin).
    2. Nothing under `mkosi.extra/` provides the extras file - everything there
       ships in EVERY image, which is the distinction the kg-probe profile in the
       same verify phase already turns on.
    3. Only a phase whose name says `verify` writes that path. A release phase
       writing it would put the admission on the shipped image while every comment
       in the tree said otherwise.

NOT covered: whether a built release image really lacks the file. That needs an
image to inspect and this runs on the tree; the guarantee comes from (2) and (3)
plus `check-verify-image.sh`, which already fails any release phase that branches
on the verify flag.

Shown to fail before being trusted: `dev/scripts/test-check-probe-admission.mjs`
plants each of the three.
"""

import pathlib
import re
import sys

ROOT = (
    pathlib.Path(sys.argv[1]).resolve()
    if len(sys.argv) > 1
    else pathlib.Path(__file__).resolve().parents[2]
)

# Ids that exist to drive the system from outside and must never be shipped as
# user surfaces. Add one here when a new probe is written; the point of naming
# them is that the refusal is about a KIND of id, not about one spelling.
PROBE_IDS = ("dogfood", "kg-probe")

EXTRAS = "user-surfaces.extra"
IDENTITY = ROOT / "sdk/permissions/src/identity.rs"
PHASES = ROOT / "dev/mkosi/mkosi.build.d"
EXTRA_TREE = ROOT / "dev/mkosi/mkosi.extra"


def compiled_in_surfaces() -> list[str]:
    """Every id listed in the compiled-in user-surface constants."""
    text = IDENTITY.read_text(encoding="utf-8", errors="replace")
    ids: list[str] = []
    for const in ("USER_SURFACES", "USER_SURFACES_DEV"):
        m = re.search(rf"const {const}[^=]*=\s*&\[(.*?)\]", text, re.S)
        if m:
            ids.extend(re.findall(r'"([^"]+)"', m.group(1)))
    return ids


def main() -> int:
    if not IDENTITY.is_file():
        print(f"NOTHING WAS READ: no {IDENTITY}", file=sys.stderr)
        return 2
    if not PHASES.is_dir():
        print(f"NOTHING WAS READ: no {PHASES}", file=sys.stderr)
        return 2

    problems: list[str] = []

    # (1) The compiled-in list is the release image's admission list.
    surfaces = compiled_in_surfaces()
    if not surfaces:
        print(
            f"NOTHING WAS READ: no user-surface constants found in {IDENTITY}",
            file=sys.stderr,
        )
        return 2
    for probe in PROBE_IDS:
        if probe in surfaces:
            problems.append(
                f"`{probe}` is a verify probe and appears in the compiled-in user "
                f"surfaces. That admits it on every image that ships, for a test's "
                f"convenience, permanently - the widening the per-variant policy "
                f"file exists to avoid."
            )

    # (2) mkosi.extra ships in every image, so the extras file cannot live there.
    if EXTRA_TREE.is_dir():
        for path in EXTRA_TREE.rglob(EXTRAS):
            problems.append(
                f"{path.relative_to(ROOT)} is under mkosi.extra, which ships in "
                f"EVERY image, so this admits the probe on release systems too. "
                f"Stage it from the verify phase into $DESTDIR instead, the way the "
                f"kg-probe profile in that phase already is."
            )

    # (3) Only a verify-named phase may write it.
    writers = []
    for phase in sorted(PHASES.iterdir()):
        if not phase.is_file():
            continue
        if EXTRAS in phase.read_text(encoding="utf-8", errors="replace"):
            writers.append(phase.name)
    for name in writers:
        if "verify" not in name:
            problems.append(
                f"{name} writes {EXTRAS} and is not a verify phase, so the "
                f"admission lands on the release image. Only a phase whose name "
                f"says verify may stage it."
            )

    if problems:
        print("probe admission:\n")
        for p in problems:
            print(f"  - {p}")
        return 1

    staged = ", ".join(writers) if writers else "no phase"
    print(
        f"{len(PROBE_IDS)} probe id(s) absent from the {len(surfaces)} compiled-in "
        f"user surfaces; {EXTRAS} staged by {staged}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
