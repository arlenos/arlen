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

The ledger below is the point of the file. It is not an exemption list - each
entry is a daemon the image does not have, with what that costs, so the number
appears in the output rather than in nobody's head. Moving an entry out is the
work. Adding one is admitting a gap, which is fine as long as it is written down
rather than discovered a year later by someone wondering why installing a
package does nothing.

Not in `just checks`, for the reason `check-invoke-scope` is not: it reports
real, pre-existing absences, and declaring them to reach a green is the wrong way
round. Run it by hand; wire it in when the list is short enough to defend.
"""

import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
IMAGE = ROOT / "dev/mkosi/mkosi.extra"
SKIP_PARTS = {"target", "mkosi.builddir", "node_modules", ".git"}

# unit -> what the image not having it costs. Written as consequence rather than
# motive: whether each absence is deliberate is not something this file can know,
# but what is missing from the running system is.
ABSENT_FROM_IMAGE = {
    "installd.service": "no package can be installed on the image",
    "install-helper.service": "the root half of installation is absent",
    "permission-helper.service": "no profile can be written by the root helper",
    "org.arlen.InstallDaemon1.service": "installd cannot be bus-activated",
    "org.arlen.InstallHelper1.service": "the install helper cannot be bus-activated",
    "org.arlen.PermissionHelper1.service": "the permission helper cannot be bus-activated",
    "xdg-desktop-portal-arlen.service": "no Arlen portal backend: file chooser and screenshot fall back",
    "org.freedesktop.impl.portal.desktop.arlen.service": "the portal backend cannot be bus-activated",
    "arlen-accountsd.service": "online accounts are unavailable",
    "org.arlen.Accounts1.service": "the accounts daemon cannot be bus-activated",
    "arlen-connectionsd.service": "remote connections are unavailable",
    "org.arlen.Connections1.service": "the connections daemon cannot be bus-activated",
    "arlen-transferd.service": "policy-gated transfers are unavailable",
    "arlen-modulesd.service": "no module (Tier 1 or Tier 2) can run",
    "arlen-settings-broker.service": "settings writes have no broker",
    "arlen-trash-cleanup.service": "staged deletions are never collected",
    "arlen-knowledge-mcp.service": "the graph MCP server is unavailable to the AI layer",
    "arlen-file-manager-mcp.service": "the file-manager MCP server is unavailable",
    "arlen-system-monitor-mcp.service": "the system-monitor MCP server is unavailable",
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
