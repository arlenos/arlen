#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Every shipped system unit is named by the cgroup resolver, or excused.

`sdk/permissions/src/unit_identity.rs` maps a system unit to the app_id its peers
authenticate as. The mapping is a hand-kept table, deliberately - what makes the
cgroup route sound is that the kernel guarantees the KEY while we choose the
VALUE, and deriving the value from the name would both give away that property and
disagree with the binary route, which resolves `arlen-graph.service` as
`knowledge` rather than `arlen-graph`.

A hand-kept table drifts. The failure is quiet in the worst direction: a new system
daemon ships, nothing adds it here, and it authenticates as nobody - which reads
from outside exactly like a daemon that is refused for a good reason. So this
checks both directions.

    missing    a shipped `.service` under the image's system unit dir with no
               entry and no excuse
    stale      an entry naming a unit the image does not ship, which would sit
               there looking like coverage
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
UNITS = ROOT / "dev/mkosi/mkosi.extra/usr/lib/systemd/system"
RESOLVER = ROOT / "sdk/permissions/src/unit_identity.rs"

# Shipped units that deliberately carry no Arlen identity, with the reason.
NOT_A_PEER = {
    "arlen-llama.service": (
        "the model server is not an Arlen component and speaks no Arlen socket; "
        "the daemon reaches it through ai-proxy's catalogue, so it never "
        "authenticates as a peer"
    ),
}


def table_entries(src: str) -> dict[str, str]:
    """The `("unit.service", "app_id")` pairs in UNIT_APP_IDS."""
    block = re.search(r"UNIT_APP_IDS: &\[\(&str, &str\)\] = &\[(.*?)\];", src, re.S)
    if not block:
        return {}
    return dict(re.findall(r'\("([^"]+)",\s*"([^"]+)"\)', block.group(1)))


def main() -> int:
    if not UNITS.is_dir():
        print(f"{Path(__file__).name}: no system unit dir at {UNITS}", file=sys.stderr)
        return 1
    entries = table_entries(RESOLVER.read_text(encoding="utf-8"))
    if not entries:
        print("could not read UNIT_APP_IDS out of the resolver", file=sys.stderr)
        return 1

    shipped = {p.name for p in UNITS.glob("*.service")}
    problems = []

    for unit in sorted(shipped):
        if unit in entries or unit in NOT_A_PEER:
            continue
        problems.append(
            f"{unit} ships but the cgroup resolver does not name it. A system "
            f"daemon with no entry authenticates as nobody, which looks from "
            f"outside exactly like one refused on purpose. Add it to "
            f"UNIT_APP_IDS, or to NOT_A_PEER with the reason it speaks no "
            f"Arlen socket."
        )

    for unit in sorted(entries):
        if unit not in shipped:
            problems.append(
                f"{unit} is mapped to '{entries[unit]}' but no such unit ships. "
                f"An entry for a unit that does not exist is coverage that "
                f"cannot fire; delete it."
            )

    for unit in sorted(NOT_A_PEER):
        if unit not in shipped:
            problems.append(f"{unit} is excused but no longer ships; delete the entry")
        elif unit in entries:
            problems.append(
                f"{unit} is both excused and mapped to '{entries[unit]}'; "
                f"the excuse outlived its reason"
            )

    if problems:
        for p in problems:
            print(f"  {p}")
        return 1

    print(
        f"{len(entries)} system unit(s) named by the cgroup resolver, "
        f"{len(NOT_A_PEER)} excused with a reason"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
