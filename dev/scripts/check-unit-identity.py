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

    missing    a shipped `.service` with no entry and no excuse
    stale      an entry naming a unit the image does not ship, which would sit
               there looking like coverage
    disagrees  a per-user entry whose id differs from what the BINARY route
               produces for the same daemon, with no recorded reason. Two
               resolvers naming one daemon differently is how a profile lookup
               silently misses - and the miss answers "no grants", which reads
               as correctly-locked-down rather than misconfigured.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
UNITS = ROOT / "dev/mkosi/mkosi.extra/usr/lib/systemd/system"
USER_UNITS = ROOT / "dev/mkosi/mkosi.extra/usr/lib/systemd/user"
RESOLVER = ROOT / "sdk/permissions/src/unit_identity.rs"

# Per-user units whose binary resolves to NO app id, so the table cannot state one
# without inventing it. Named rather than omitted: a component we cannot name is
# one we cannot grant to, and leaving it out of both lists would make the gap look
# like coverage.
UNNAMEABLE = {
    "arlen-store-backend.service": (
        "/usr/lib/arlen/libexec/arlen-store-backend matches no identity rule "
        "either; same shape as the signer above"
    ),
}

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


def exec_start_binary(unit_file: Path) -> str:
    """The first word of the unit's ExecStart, or "" when it declares none."""
    for line in unit_file.read_text(encoding="utf-8", errors="replace").splitlines():
        if line.startswith("ExecStart="):
            return line[len("ExecStart="):].split()[0]
    return ""


def derived_from_binary(exe: str) -> str:
    """The app id the BINARY route produces, by the convention it encodes.

    Not a reimplementation of `path_to_app_id` - the point is narrower. Every
    shipped daemon is `<dir>/arlen-<name>` and resolves to `<name>`, so asserting
    that convention costs one line and forces every DEVIATION to be written down
    with its reason. The deviation is the whole value: `arlen-ai-engine-daemon`
    resolves to `ai-agent`, and a name-derived guess would have produced an id no
    profile is filed under - a lookup answering "no grants", which reads as
    correctly-locked-down rather than misconfigured.
    """
    return exe.rsplit("/", 1)[-1].removeprefix("arlen-")


# Table entries whose id deliberately differs from the binary convention, each
# with the reason. Anything not listed here must agree.
BINARY_DEVIATIONS = {
    "arlen-graph.service": (
        "knowledge",
        "the daemon's binary is `arlen-graph-daemon` and its id is `knowledge`, a "
        "split the resolver already carries explicitly (an arm in identity.rs maps "
        "/usr/bin/arlen-graph-daemon -> knowledge) - so both resolvers DO agree; "
        "what disagrees is the binary's name, which is the graph-versus-knowledge "
        "naming question this tree has carried for months and which is Tim's to "
        "settle. Recording it here rather than renaming either side keeps the "
        "deviation visible instead of resolving it by accident",
    ),
    "arlen-ai-engine-daemon.service": (
        "ai-agent",
        "the daemon is the AI agent; `ai-engine-daemon` is the unit's name for it "
        "and nothing else in the system uses that id",
    ),
}


def user_table_entries(src: str) -> dict[str, str]:
    """The `("unit.service", "app_id")` pairs in USER_UNIT_APP_IDS."""
    block = re.search(r"USER_UNIT_APP_IDS: &\[\(&str, &str\)\] = &\[(.*?)\];", src, re.S)
    if not block:
        return {}
    return dict(re.findall(r'\("([^"]+)",\s*"([^"]+)"\)', block.group(1)))


def check_user_units(src: str) -> list[str]:
    """Both directions for the per-user table, plus agreement with the binary."""
    if not USER_UNITS.is_dir():
        return [f"no user unit dir at {USER_UNITS}"]
    entries = user_table_entries(src)
    if not entries:
        return ["could not read USER_UNIT_APP_IDS out of the resolver"]
    shipped = {p.name for p in USER_UNITS.glob("*.service")}
    problems = []

    for unit in sorted(shipped):
        if unit in UNNAMEABLE:
            if unit in entries:
                problems.append(
                    f"{unit} is listed as unnameable but the table names it "
                    f"'{entries[unit]}'; the gap was closed and the entry left behind"
                )
            continue
        if unit not in entries:
            problems.append(
                f"{unit} ships as a per-user unit but the table does not name it. "
                f"The launcher would register nothing for it, and its peers would "
                f"be refused with no way to tell that from a policy decision. Add "
                f"it, or record it in UNNAMEABLE with why it has no id."
            )
            continue
        exe = exec_start_binary(USER_UNITS / unit)
        want, why = BINARY_DEVIATIONS.get(unit, (None, None))
        if want is not None:
            if entries[unit] != want:
                problems.append(
                    f"{unit} is recorded as deviating to '{want}' ({why}) but the "
                    f"table says '{entries[unit]}'"
                )
        elif exe and entries[unit] != derived_from_binary(exe):
            problems.append(
                f"{unit} maps to '{entries[unit]}' but its binary {exe} resolves "
                f"to '{derived_from_binary(exe)}'. Two resolvers naming one daemon "
                f"differently is how a profile lookup silently misses. Fix the "
                f"table, or record the deviation in BINARY_DEVIATIONS with why."
            )

    for unit in sorted(entries):
        if unit not in shipped:
            problems.append(
                f"{unit} is mapped to '{entries[unit]}' but no such user unit "
                f"ships; delete the entry"
            )
    for unit in sorted(UNNAMEABLE):
        if unit not in shipped:
            problems.append(f"{unit} is listed as unnameable but no longer ships")
    return problems


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

    src = RESOLVER.read_text(encoding="utf-8")
    problems.extend(check_user_units(src))

    if problems:
        for p in problems:
            print(f"  {p}")
        return 1

    print(
        f"{len(entries)} system unit(s) named by the cgroup resolver, "
        f"{len(NOT_A_PEER)} excused with a reason; "
        f"{len(user_table_entries(src))} per-user unit(s) named by the launcher "
        f"table and agreeing with the binary route, {len(UNNAMEABLE)} with no id yet"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
