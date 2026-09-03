#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Check that a unit which exists twice says the same thing in both places.

Most daemons here carry their systemd unit in their own `dist/` directory, next to
the code that explains it. The image ships units from `mkosi.extra`. For several
daemons BOTH exist - the same file name, two copies, one of which is deployed and
one of which is read.

`08s-session.sh.chroot` states the rule this check enforces, in its own words and
about a different pair: "two artefacts at one path is how the wrong one ends up
running". Measured on 3 September, six of the twenty-three shipped user units
differed from their crate copy. Five differed only in comment wording. One did
not: `arlen-graph.service` on the image sets
`Environment=XDG_CONFIG_HOME=/usr/share/arlen-config` and the crate copy did not,
so the knowledge daemon read its watch config from a different place than anybody
editing the obvious file would have believed. An edit to the crate copy changed
nothing on a machine, silently, for as long as that lasted.

DIRECTIVES, NOT COMMENTS. The two copies are allowed to explain themselves
differently - a crate's copy sits next to the code and an image's next to its
siblings, and forcing byte-identity would mean churn on every wording change with
no behaviour behind it. What may not differ is what systemd acts on.

The real fix is one copy rather than two, and that is a convention decision
(installd, capsuled and the undo signer already install theirs FROM `dist/` in
their build step, so the tree holds both answers). Until somebody settles it, the
copies must at least agree.
"""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = (
    Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
)
SHIPPED = ROOT / "dev/mkosi/mkosi.extra/usr/lib/systemd"
#: The OTHER artefact kept in two places and read by a daemon starter: a D-Bus
#: activation file. Same trap, same rule - and the same stakes, since one of these
#: was missing its `SystemdService=` line in August, which meant activation started
#: the bare binary and skipped the unit's whole sandbox.
DBUS = ROOT / "dev/mkosi/mkosi.extra/usr/share/dbus-1/services"
#: Where a crate keeps its own copy. Every component root that has daemons in it.
CRATE_ROOTS = ("daemons", "ai", "apps", "contracts", "sdk")


#: Pairs whose directives differ and where reconciling them is a DECISION rather
#: than a typo. One entry, and it is worth reading rather than skipping: the
#: config broker's crate copy runs it as a dedicated `User=arlen-config` with
#: ProtectHome, PrivateDevices and ProtectProc=invisible, and its own Description
#: calls it "the separate-uid owner of the AI master switches". The copy the image
#: ships runs it as `User=root` with a smaller hardening set and an
#: `ARLEN_CONFIG_BROKER_OWNER_UID` instead. So the separate-uid boundary the design
#: rests on is not the one deployed, and no user named `arlen-config` is created
#: anywhere in the image - pointing the unit at one would fail to start.
#:
#: MAY SHRINK, MAY NOT GROW. Closing it means deciding which is true: create the
#: uid and ship the hardened unit, or accept root-with-owner-uid and say so in the
#: crate copy. That is a security-model call, not a checker's.
DECISION_PENDING: dict[str, str] = {
    "arlen-config-broker.service": (
        "the crate copy runs a dedicated uid the image does not create, and the "
        "image runs it as root. Which is the design is still a decision, but it is "
        "no longer a symmetric one, because the crate copy AS WRITTEN cannot work. "
        "`server.rs::owner_uid` says the separate-uid deployment needs "
        "ARLEN_CONFIG_BROKER_OWNER_UID to carry the SESSION user's uid, 'set by the "
        "systemd unit', and falls back to the broker's own uid without it - so a "
        "broker running as `arlen-config` with no override would expect every peer "
        "to be `arlen-config`, and every legitimate caller (Settings, the AI daemon) "
        "runs as the session user. All of them refused. The crate copy sets no "
        "override. It also has EnvironmentFile=/etc/arlen/config-broker.env with no "
        "leading `-`, and no phase stages that file, so the unit would fail to start "
        "before reaching any of it. The image copy is coherent today: root, "
        "OWNER_UID=1000, callers accepted. Its one dead line is the "
        "SystemCallFilter re-allowing the three landlock calls - nothing in the "
        "crate calls landlock. So the separate-uid target costs three things the "
        "crate copy does not have (the user, the OWNER_UID environment, the env "
        "file or a `-` on it), and that is the shape of the decision"
    ),
}


def directives(path: Path) -> list[str]:
    """The lines systemd acts on: no blanks, no comments."""
    out = []
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        stripped = line.strip()
        if stripped and not stripped.startswith("#"):
            out.append(stripped)
    return out


def crate_copies(name: str) -> list[Path]:
    """Every `dist/<name>` under a component root."""
    found: list[Path] = []
    for root in CRATE_ROOTS:
        base = ROOT / root
        if not base.is_dir():
            continue
        for depth in ("*/dist", "*/*/dist"):
            found.extend(p for p in base.glob(f"{depth}/{name}") if p.is_file())
    return sorted(found)


def main() -> int:
    if not SHIPPED.is_dir():
        print(f"{SHIPPED.relative_to(ROOT)} is missing; the image ships no units")
        return 1

    shipped = sorted(
        p for sub in ("user", "system") for p in (SHIPPED / sub).glob("*.service")
    ) + sorted(DBUS.glob("*.service") if DBUS.is_dir() else [])
    if not shipped:
        print("NOTHING WAS READ: the image ships no unit files", file=sys.stderr)
        return 2

    problems: list[str] = []
    compared = 0
    for unit in shipped:
        for copy in crate_copies(unit.name):
            compared += 1
            theirs, ours = directives(copy), directives(unit)
            if theirs == ours:
                if unit.name in DECISION_PENDING:
                    problems.append(
                        f"{unit.name} is in DECISION_PENDING and its two copies now "
                        f"agree. Drop the entry: a list of settled questions makes "
                        f"the open ones harder to find."
                    )
                continue
            if unit.name in DECISION_PENDING:
                continue
            only_copy = [d for d in theirs if d not in ours]
            only_image = [d for d in ours if d not in theirs]
            detail = "".join(
                f"\n      only in {copy.relative_to(ROOT)}: {d}" for d in only_copy
            ) + "".join(
                f"\n      only on the image:            {d}" for d in only_image
            )
            problems.append(
                f"{unit.name} says different things in its two copies. The image ships "
                f"{unit.relative_to(ROOT)} and the crate keeps {copy.relative_to(ROOT)}; "
                f"an edit to the second changes nothing on a machine.{detail}"
            )

    if problems:
        for p in problems:
            print(p)
        return 1

    print(
        f"{len(shipped)} shipped unit and activation file(s), {compared} of them kept "
        f"in a crate too, each pair agreeing on every directive"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
