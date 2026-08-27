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
PATH_RESOLVER = ROOT / "sdk/permissions/src/identity.rs"

# Units whose ExecStart binary `path_to_app_id` cannot name, with the reason.
# MAY SHRINK, MAY NOT GROW.
#
# Separate from NOT_A_PEER (which excuses a unit from the TABLE) and from
# UNNAMEABLE (which excuses it from having an id at all): this list is about the
# BINARY route. A daemon can be named by the unit table, because the supervisor
# stamps its cgroup, and still resolve to `UnknownBinary` when it connects to
# someone else's socket and is identified by `/proc/pid/exe` instead.
UNRESOLVED_BINARY = {
    "arlen-event-bus.service": (
        "the binary is `/usr/bin/event-bus` with no `arlen-` prefix, so rule 2 "
        "misses it. It IS a client - the per-user bus forwards to the system bus - "
        "and resolves there as UnknownBinary, which the bus reads as an undeclared "
        "scope and therefore the machine-wide view. It works because the identity "
        "failed to resolve, not because anything granted it. Closing it is a "
        "naming decision: rename to `arlen-event-bus` for rule 2, or add an arm"
    ),
    "arlen-event-bus-system.service": (
        "the same binary as arlen-event-bus.service, under the system manager"
    ),
    "arlen-store-backend.service": (
        "same shape as its UNNAMEABLE entry: no identity rule matches the binary"
    ),
}

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



# Units written by a BUILD PHASE rather than shipped in `mkosi.extra`, each with
# the phase that writes it. They still need a table entry: with the `/proc`
# fallback removed, a caller the broker cannot name is refused outright, so a unit
# that talks to anything has to be nameable whether it ships from the extra tree
# or from a phase.
#
# The entry is not taken on trust - the phase named here must actually write the
# unit, checked below. An exception that cannot expire is the thing this file
# exists to prevent, and both entries here are conditional: one ships only on
# verify images, the other only when the kernel layer is asked for.
PHASE_WRITTEN = {
    "arlen-kg-probe.service": "dev/mkosi/mkosi.build.d/09-verify-probes.sh.chroot",
    "arlen-kernel-layer.service": "dev/mkosi/mkosi.build.d/07-kernel-layer.sh.chroot",
}


def phase_written_unit_is_written(repo, unit: str) -> bool:
    """Whether the build phase claimed for `unit` really writes it."""
    phase = repo / PHASE_WRITTEN[unit]
    if not phase.is_file():
        return False
    return unit in phase.read_text(encoding="utf-8", errors="replace")


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
            if unit in PHASE_WRITTEN:
                if phase_written_unit_is_written(ROOT, unit):
                    continue
                problems.append(
                    f"{unit} is carried as written by {PHASE_WRITTEN[unit]}, but "
                    f"that phase does not write it. Either the phase changed or "
                    f"the entry is stale."
                )
                continue
            problems.append(
                f"{unit} is mapped to '{entries[unit]}' but no such user unit "
                f"ships; delete the entry"
            )
    for unit in sorted(UNNAMEABLE):
        if unit not in shipped:
            problems.append(f"{unit} is listed as unnameable but no longer ships")
    return problems


def resolver_named_paths() -> set[str]:
    """Absolute binary paths `path_to_app_id` names by strict equality (rule 1)."""
    src = PATH_RESOLVER.read_text(encoding="utf-8")
    return set(
        re.findall(r'"(/usr/(?:lib/arlen/libexec|bin)/[A-Za-z0-9._-]+)"', src)
    )


def rule_two_names(exe: str) -> bool:
    """Whether rule 2 (`/usr/bin/arlen-{name}` -> `{name}`) covers this path."""
    rest = exe.removeprefix("/usr/bin/arlen-")
    if rest == exe or not rest:
        return False
    return all(c.islower() or c.isdigit() or c in "._-" for c in rest)


def check_binaries_resolvable() -> list[str]:
    """Every shipped unit's binary is one `path_to_app_id` can name.

    The check `derived_from_binary` makes is about the CONVENTION - that a
    table id matches the `arlen-<name>` shape - and it says so. This is the
    other question, which nothing asked: does the resolver name that binary at
    ALL. A daemon whose path matches no rule resolves to `UnknownBinary` when a
    socket peer identifies it, gets no profile, and is refused or - worse, on the
    event bus - falls through to an undeclared scope and the machine-wide view.
    """
    if not PATH_RESOLVER.is_file():
        # A fixture tree that is exercising the table checks and ships no
        # resolver. Saying nothing beats inventing a finding about a file the
        # case under test does not have.
        return []
    named = resolver_named_paths()
    if not named:
        return ["identity.rs names no strict-equality binary path at all"]
    problems = []
    for base in (UNITS, USER_UNITS):
        if not base.is_dir():
            continue
        for unit_file in sorted(base.glob("*.service")):
            unit = unit_file.name
            exe = exec_start_binary(unit_file)
            if not exe.startswith("/"):
                continue
            if exe in named or rule_two_names(exe):
                if unit in UNRESOLVED_BINARY:
                    problems.append(
                        f"{unit} is listed as unresolvable but `{exe}` is named "
                        f"now; the gap was closed and the entry left behind"
                    )
                continue
            if unit in UNRESOLVED_BINARY or unit in NOT_A_PEER:
                continue
            problems.append(
                f"{unit} runs `{exe}`, which `path_to_app_id` names nothing for. "
                f"A peer identified by /proc/pid/exe resolves it as UnknownBinary: "
                f"no profile, and on a socket that treats an undeclared scope as "
                f"unrestricted it reads as a grant. Add a rule, rename the binary "
                f"under `/usr/bin/arlen-`, or record it in UNRESOLVED_BINARY."
            )
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
            # Same allowance as the user-unit loop above: a unit written by a
            # build phase is not in the extra tree, and the phase must really
            # write it.
            if unit in PHASE_WRITTEN:
                if phase_written_unit_is_written(ROOT, unit):
                    continue
                problems.append(
                    f"{unit} is carried as written by {PHASE_WRITTEN[unit]}, but "
                    f"that phase does not write it. Either the phase changed or "
                    f"the entry is stale."
                )
                continue
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
    problems.extend(check_binaries_resolvable())

    if problems:
        for p in problems:
            print(f"  {p}")
        return 1

    print(
        f"{len(entries)} system unit(s) named by the cgroup resolver, "
        f"{len(NOT_A_PEER)} excused with a reason; "
        f"{len(user_table_entries(src))} per-user unit(s) named by the launcher "
        f"table and agreeing with the binary route, {len(UNNAMEABLE)} with no id "
        f"yet; every shipped binary is one the path resolver names, "
        f"{len(UNRESOLVED_BINARY)} recorded as not"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
