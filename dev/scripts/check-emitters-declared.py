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

# The resolver's own match arms: `"/usr/lib/..." => { return Ok("id"...`. Read
# from the arms rather than the test table below them, because the table is a
# SAMPLE - `arlen-auditd` has an arm and no table entry, and trusting the table
# reported the audit daemon as unresolvable when the running system resolves it
# perfectly well.
ARM = re.compile(r'"(?P<path>/usr/[^"]+)"\s*=>\s*\{\s*return\s+Ok\("(?P<id>[a-z0-9.\-]+)"', re.S)

# Components whose directory name is not the id they resolve to.
DIR_TO_ID = {
    "daemons/audit-daemon": "auditd",
    "daemons/notification-daemon": "notifyd",
    "daemons/anomaly-detector": "anomalyd",
    "daemons/power-daemon": "powerd",
}

# Emitters that are not in the image, with the reason.
NOT_SHIPPED = {
    "daemons/modulesd": "the module runtime is a later phase (see check-shipped-units.py)",
}

SKIP = ("/target/", "node_modules", "/.git/")


def emitters(repo: Path) -> dict[str, set[str]]:
    """Component directory -> the bus topics its source emits."""
    out: dict[str, set[str]] = {}
    for root in ("daemons", "apps"):
        for d in sorted((repo / root).glob("*")):
            if not d.is_dir():
                continue
            topics = set()
            for f in d.rglob("*.rs"):
                if any(s in str(f) for s in SKIP):
                    continue
                for m in PUBLISH_CALL.finditer(f.read_text(encoding="utf-8", errors="replace")):
                    t = m.group("topic")
                    # A dotted bus topic, not a Tauri window event or a log line
                    # behind a method that happens to be called `emit`.
                    if "." in t and "://" not in t and " " not in t:
                        topics.add(t)
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
