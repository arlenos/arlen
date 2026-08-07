# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a service unit written in the repo reaches the image.

A daemon is finished when it runs on the system, not when it compiles. Every
`daemons/*/dist/*.service` is a statement that something should run; the image
only runs what `dev/mkosi/mkosi.extra` installs and what a `mkosi.build.d` phase
stages the binary for. A unit in the first set and not the second is a daemon
that exists in the repository and nowhere else - which reads, from every plan
and every status line, exactly like a daemon that is deployed.

This was found by looking: `arlen-wallpaperd.service` had been written, the
renderer built, and no phase ever compiled it into the image. The desktop had no
wallpaper and nothing said so. Nineteen more units were in the same state.

The ledger below is the point of the file. Each entry is a daemon the image does
not have and the reason it does not, so a unit can only be absent on purpose.
Adding an entry means writing down a decision; the check fails on any absence
nobody has.

Not in `just checks`, for the reason `check-invoke-scope` is not: it reports
real, pre-existing absences, and declaring them to reach a green is the wrong way
round. Run it by hand; wire it in when the list is short enough to defend.
"""

import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
IMAGE = ROOT / "dev/mkosi/mkosi.extra"
SKIP_PARTS = {"target", "mkosi.builddir", "node_modules", ".git"}

# unit -> why the image does not have it. Every entry here is a scope call the
# planner already made and re-confirmed (coder-jobs 14 Jul, re-swept 21 Jul); the
# reasons are quoted from it rather than reconstructed, so reading this file
# cannot re-open a settled question. That is the failure this text exists to
# prevent: the first version listed only the consequence of each absence, which
# reads as nineteen open holes and invites someone to "fix" a deliberate scope.
#
# The absences are deliberate. A unit that is NOT here and not shipped is the
# real finding - that was `arlen-wallpaperd`, which had a unit, a working
# renderer and no build phase, and it is now staged.
ABSENT_FROM_IMAGE = {
    # D-Bus-root-activated, not boot-verified: out of minimal-image scope.
    "installd.service": "installation is D-Bus-root-activated, not boot-verified",
    "install-helper.service": "the root half of the same deferral",
    "permission-helper.service": "the root profile writer, same deferral",
    "org.arlen.InstallDaemon1.service": "activation file for the above",
    "org.arlen.InstallHelper1.service": "activation file for the above",
    "org.arlen.PermissionHelper1.service": "activation file for the above",
    # Deferred to the portal/capture strand: the portal is immature and its
    # screen-capture half carries open decisions. Verified 8 Aug that no staged
    # app invokes a file picker or screenshot, so the deferral costs nothing yet.
    "xdg-desktop-portal-arlen.service": "deferred to the portal/capture strand",
    "org.freedesktop.impl.portal.desktop.arlen.service": "activation file for the above",
    # Outside the minimal dogfood appliance (desktop + 4 apps + KG-AI + consent).
    "arlen-accountsd.service": "online accounts are outside the appliance scope",
    "org.arlen.Accounts1.service": "activation file for the above",
    "arlen-connectionsd.service": "no packaged consumer yet",
    "org.arlen.Connections1.service": "activation file for the above",
    "arlen-transferd.service": "no packaged consumer yet",
    "arlen-modulesd.service": "no packaged consumer yet",
    "arlen-settings-broker.service": "settings is not a staged app",
    "arlen-trash-cleanup.service": "outside the appliance scope",
    # AI-spawned on demand rather than session services: a missing one is a
    # capability the AI layer reports as unavailable, not a broken boot.
    "arlen-knowledge-mcp.service": "MCP servers are AI-spawned on demand",
    "arlen-file-manager-mcp.service": "MCP servers are AI-spawned on demand",
    "arlen-system-monitor-mcp.service": "MCP servers are AI-spawned on demand",
}


def repo_units() -> set[str]:
    """Every service unit written under a `dist/` directory, at any depth.

    At any depth is the point: the portal keeps its two units in `dist/systemd/`
    and `dist/dbus/` rather than directly in `dist/`, and a `dist/*.service` glob
    silently misses them. The first version of this file did exactly that and
    then reported both as ledger entries for units that do not exist.
    """
    out = set()
    for path in ROOT.rglob("*.service"):
        if SKIP_PARTS & set(path.parts) or "dist" not in path.parts:
            continue
        out.add(path.name)
    return out


def image_units() -> set[str]:
    """Every service unit the image tree installs, from ANY location.

    All of them: systemd system units, systemd user units and D-Bus activation
    files live in three different directories, and comparing against only one is
    how a unit that IS shipped gets reported as missing. That mistake was made
    while writing this file.
    """
    return {p.name for p in IMAGE.rglob("*.service")}


def main() -> int:
    written, shipped = repo_units(), image_units()
    if not written:
        print("found no dist/*.service units; the check needs updating")
        return 2

    missing = sorted(written - shipped)
    undeclared = [u for u in missing if u not in ABSENT_FROM_IMAGE]
    # Two different ways a ledger entry stops being true, and they need different
    # words: the image gained the unit (good, drop the entry) or the repository
    # never had it under that name (the entry is a typo, or the unit was renamed).
    # Collapsing them says "the image now installs it" about a unit nobody wrote.
    stale = sorted(u for u in ABSENT_FROM_IMAGE if u in written and u in shipped)
    unknown = sorted(u for u in ABSENT_FROM_IMAGE if u not in written)

    if undeclared:
        print("units written in the repo that the image never installs:\n")
        for unit in undeclared:
            print(
                f"  - {unit}: install it under dev/mkosi/mkosi.extra and stage its "
                f"binary from a mkosi.build.d phase, or record here what its absence costs"
            )
        return 1

    if stale:
        print("units listed as absent that the image now installs; drop them from the ledger:\n")
        for unit in stale:
            print(f"  - {unit}")
        return 1

    if unknown:
        print("ledger entries naming a unit no dist/ directory writes:\n")
        for unit in unknown:
            print(f"  - {unit}: renamed, removed, or misspelled here")
        return 1

    print(f"{len(written)} unit(s) written, {len(written) - len(missing)} reach the image")
    print(f"{len(missing)} do not, each named in this file:")
    for unit in missing:
        print(f"  - {unit}: {ABSENT_FROM_IMAGE[unit]}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
