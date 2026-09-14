#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""A component that emits onto the bus needs a profile the bus can find.

Publish exemption stopped following the install tier on 14 Aug: only the named
originators (the compositor, the kernel layer) publish freely, and everything else
is held to its declared list. A producer with no profile therefore declares
nothing, which means it is denied everything - and a denied publish is a
`continue` in the producer loop against a fire-and-forget protocol, so the
emitting code sees success while every consumer of that topic goes quiet.

WHY THIS EXISTS RATHER THAN A BOOT. Three oracles were needed for three producers
on the day the rule changed, and neither of the others covers this case:

  * the static scan in `check-subscribe-scope.py` reads emit arguments, and found
    the topics written as literals;
  * an enforce boot found the ones composed at runtime, built as struct literals
    or emitted through a plugin - but ONLY for topics something actually sent
    during that boot;
  * neither can see a producer whose trigger the boot never reaches.

`arlen-powerd` is the third kind and the reason this file exists. It publishes
`power.state` when UPower reports a change, and a QEMU guest has no battery - so
it never connects as a producer and every enforce boot looks clean, while on a
laptop it would be denied and the battery indicator would stop updating. Works in
the VM, breaks on metal, silent on both.

IT LOOKS UNDER `dev/` SINCE 14 SEPTEMBER, and did not before. Two dogfood tools
ship from there (`arlen-event-emit`, `arlen-dogfood`, build phase 09-dogfood,
both resolvable as ids), and one of them had no profile at all - so under
enforcement the injector whose whole job is to prove an event was delivered would
have gone on printing its confirmation while the bus dropped everything it sent.
Nothing asked for that profile because this scan walked `daemons/` and `apps/`
only.

AND IT NO LONGER READS THE EMITS ITSELF. `check-subscribe-scope.py` asks the
other half of the same question and grew two shapes this file did not have on the
same day - a constant resolved across the crate rather than within one file, and
a topic returned from a function the emit names. One detector, imported; two
copies would leave one of them a shape behind, which is how the sentinel's
constant survived here until September.

IT DELIBERATELY DOES NOT READ THE BUILD STEPS, which is worth knowing before
extending it. The obvious design asks "is this component installed?" so an
unshipped daemon is not a false alarm - and I wrote that four times, each version
confidently wrong in a different way. The steps wrap `install` over a line
continuation; they name their crate by `--manifest-path` for cargo and not at all
for the Tauri apps; a step that merely MENTIONS another component's path claims
its destination. The last cut attributed the file manager's binary to the undo
signer, which is the kind of confident nonsense that sends somebody to write a
profile under the wrong id.

So the shipping question is a short hand list with a reason per entry. Worse in
principle, better in practice: it is small, every entry is checkable by eye, and
being wrong is loud rather than plausible. A single declared component-to-install
table in the tree would let this be derived properly, and is the right fix if the
list ever grows.
"""

import importlib.util
import re
import sys
from pathlib import Path

OWN_TREE = len(sys.argv) <= 1
REPO = Path(__file__).resolve().parents[2] if OWN_TREE else Path(sys.argv[1]).resolve()

PROFILES = "dev/mkosi/mkosi.extra/var/lib/arlen/permissions"
IDENTITY = "sdk/permissions/src/identity.rs"

# The same emit shapes `check-subscribe-scope.py` reads: the SDK emitter's method
# and the shell's hand-rolled helper.
PUBLISH_CALL = re.compile(r'(?:emit_to_event_bus|\.emit|emit_event)\s*\(\s*"(?P<topic>[^"]+)"')

#: The same call with the topic held in a CONSTANT rather than written inline:
#: `publish(events, RECORDING_EVENT, payload)`. Added 13 September after the
#: privacy sentinel emitted two topics this file could not see - it read the
#: daemon as emitting nothing, asked for no profile, and under enforcement both
#: events would have been dropped at the bus in silence. A gate matches the shape
#: its author last happened to write, and the shape here was one indirection.
PUBLISH_VIA_CONST = re.compile(
    r'(?:emit_to_event_bus|\.emit|emit_event|publish)\s*\([^)]*?\b(?P<name>[A-Z][A-Z0-9_]{2,})\b'
)

#: A `const NAME: &str = "a.topic";` the call above can be naming. Only dotted
#: values count, so a constant holding a socket name or a bus path is not read as
#: a topic.
CONST_TOPIC = re.compile(
    r'const\s+(?P<name>[A-Z][A-Z0-9_]{2,})\s*:\s*&\s*str\s*=\s*"(?P<topic>[^"\s]+\.[^"\s]+)"'
)

# The resolver's own match arms: `"/usr/lib/..." => { return Ok("id"...`. Read
# from the arms rather than the test table below them, because the table is a
# SAMPLE - `arlen-auditd` has an arm and no table entry, and trusting the table
# reported the audit daemon as unresolvable when the running system resolves it
# perfectly well.
ARM = re.compile(r'"(?P<path>/usr/[^"]+)"\s*=>\s*\{\s*return\s+Ok\("(?P<id>[a-z0-9.\-]+)"', re.S)

# Components whose directory name is not the id they resolve to, or whose binary
# name keeps the arm lookup from finding it.
#
# `knowledge` is in here despite matching its directory, and that is the point:
# its binary is `arlen-graph-daemon`, so no arm lookup keyed on the directory name
# can reach the `("/usr/bin/arlen-graph-daemon", "knowledge")` override. Without
# an entry it fell through to the bare-directory fallback and got the right answer
# by coincidence - which would have held until the day the id and the directory
# diverged, and then reported a missing profile under a name nobody uses.
DIR_TO_ID = {
    "daemons/audit-daemon": "auditd",
    "daemons/notification-daemon": "notifyd",
    "daemons/anomaly-detector": "anomalyd",
    "daemons/power-daemon": "powerd",
    "daemons/knowledge": "knowledge",
    # The binary is `arlen-calendard`, one letter off the directory, so the
    # arm-matching below (which looks for `/arlen-<dir>`) cannot bridge it and
    # would report a missing `calendar.toml` - a name nothing resolves to, which
    # is the mistake this table's own comment warns about.
    "daemons/calendar": "calendard",
}

# Emitters that are not in the image, with the reason.
NOT_SHIPPED: dict[str, str] = {
    # Empty from 15 Aug, when `modulesd` shipped, until this scan learned to look
    # under `dev/` on 14 September. The two dogfood tools there DO ship and now
    # carry profiles; the integration harness does not and cannot - it is a cargo
    # test target, it emits its synthetic events from inside the test process, and
    # the id it would be filed under is the hash-suffixed `dev.<test>` a test
    # binary resolves to, which is not a name a profile can be written for.
    "dev/integration": "the IT-1 harness is a cargo test target, never staged, and "
    "resolves to a hash-suffixed test id no profile can name",
}

SKIP = ("/target/", "node_modules", "/.git/")


def sibling_publishes(directory: Path) -> set[str]:
    """Every topic `directory` emits, read by `check-subscribe-scope.py`.

    That file owns the three shapes a topic can reach an emit by - a literal, a
    constant, a function the call names - and keeping a second reading here is
    how one of them ends up a shape behind. Loading it is safe: its module level
    reads `sys.argv` only to resolve a repo root, which is the same argument
    this gate takes.
    """
    spec = importlib.util.spec_from_file_location(
        "subscribe_scope", Path(__file__).resolve().parent / "check-subscribe-scope.py"
    )
    if spec is None or spec.loader is None:
        print("NOTHING WAS READ: cannot load check-subscribe-scope.py", file=sys.stderr)
        raise SystemExit(2)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.publishes_of(directory)


def emitters(repo: Path) -> dict[str, set[str]]:
    """Component directory -> the bus topics its source emits."""
    out: dict[str, set[str]] = {}
    for root in ("daemons", "apps", "dev"):
        for d in sorted((repo / root).glob("*")):
            if not d.is_dir():
                continue
            # ONE DETECTOR, TWO GATES. `check-subscribe-scope.py` asks the other
            # half of this question - whether a declared topic has a producer
            # behind it - so it needs the same reading of what a component emits,
            # and it grew two shapes on 14 September this file did not have: a
            # topic behind a CONSTANT resolved across the crate rather than only
            # within one file, and a topic returned from a FUNCTION the emit
            # names. Two copies of that would drift on one side, which is the
            # rule `check-profile-principals` already follows by importing from
            # `check-admitted-ids-exist`. Imported, not restated.
            topics = sibling_publishes(d)
            if topics:
                out[str(d.relative_to(repo))] = topics
    return out


def resolver_arms(repo: Path) -> dict[str, str]:
    """Install path -> app id, from the resolver's own match arms."""
    text = (repo / IDENTITY).read_text(encoding="utf-8", errors="replace")
    return {m.group("path"): m.group("id") for m in ARM.finditer(text)}


def component_id(comp: str, arms: dict[str, str]) -> str:
    """The app id a component resolves to."""
    root, name = comp.split("/", 1)
    if root == "apps":
        return f"dev.arlen.{name}"
    if comp in DIR_TO_ID:
        return DIR_TO_ID[comp]
    for path, app_id in arms.items():
        if path.endswith(f"/arlen-{name}") or path.endswith(f"/{name}"):
            return app_id
    return name


def main() -> int:
    emits = emitters(REPO)
    if not emits:
        print(f"NOTHING WAS READ: no component under {REPO} emits a bus topic", file=sys.stderr)
        return 2

    profiles = {p.stem for p in (REPO / PROFILES).glob("*/*.toml")}
    if not profiles:
        print(f"NOTHING WAS READ: no profile under {REPO}/{PROFILES}", file=sys.stderr)
        return 2

    arms = resolver_arms(REPO)
    problems: list[str] = []
    waiting: list[str] = []
    covered = 0

    for comp, topics in sorted(emits.items()):
        listed = ", ".join(sorted(topics))
        if comp in NOT_SHIPPED:
            waiting.append(
                f"{comp}: emits {listed} - {NOT_SHIPPED[comp]}. "
                f"It needs a profile the day it ships."
            )
            continue
        app_id = component_id(comp, arms)
        if app_id in profiles:
            covered += 1
            continue
        problems.append(
            f"{comp}: emits {listed} and no profile is named `{app_id}.toml`.\n"
            f"    With no profile it declares no publish scope, so under "
            f"enforcement every one of those events is dropped at the bus, and the "
            f"producer is never told because the wire protocol is fire-and-forget. "
            f"The emitting code keeps looking like it works while every consumer "
            f"goes quiet."
        )

    if waiting:
        print("emits, and is not in the image yet:")
        for line in waiting:
            print(f"  {line}")
        print()

    if problems:
        print("a bus producer whose events would be dropped:")
        for p in problems:
            print(f"  {p}")
        return 1

    print(f"OK: {covered} emitter(s), each with a profile under the id it resolves to")
    return 0


if __name__ == "__main__":
    sys.exit(main())
