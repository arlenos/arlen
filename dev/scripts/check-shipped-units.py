# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a component shipping a systemd unit either gets installed or says why not.

A daemon can be finished - key custody, protocol, tests, a hardened unit in its
own `dist/` - and still be absent from the image, because nothing connects the two
and nothing complains. That is not hypothetical: `arlen-ai-undo-signer` is built
and deployed nowhere while THREE producers write to its socket, so every
reversible action they take is journalled into nothing. The code is right and the
delivery silently is not, which is the same shape as a unit whose ReadWritePaths
omits the directory it writes, or an activation file that skips its hardening.

The gate does not demand that everything ship. Plenty of components are ahead of
the image on purpose. It demands that the answer be WRITTEN DOWN: each unit is
either installed, or listed below with the reason it waits. A silence becomes a
sentence somebody can disagree with.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
EXTRA = ROOT / "dev/mkosi/mkosi.extra/usr/lib/systemd"
BUILD_STEPS = ROOT / "dev/mkosi/mkosi.build.d"

# unit -> why it is not on the image. Each line is a CLAIM, not a shrug: if it is
# wrong, it is wrong somewhere a reader can see it. "Unreviewed" is an honest
# entry; an invented rationale is not.
NOT_YET_DEPLOYED: dict[str, str] = {
    "arlen-accountsd.service": "online-accounts is not part of the image scope yet",
    "arlen-connectionsd.service": "connections daemon is not part of the image scope yet",
    "arlen-transferd.service": "transfer daemon is not part of the image scope yet",
    "arlen-settings-broker.service": "settings broker is not part of the image scope yet",
    "arlen-modulesd.service": "the module runtime is a later phase",
    "arlen-trash-cleanup.service": "trash retention timer, unreviewed for deployment",
    "arlen-file-manager-mcp.service": "MCP servers are not staged into the image yet",
    "arlen-knowledge-mcp.service": "MCP servers are not staged into the image yet",
    "arlen-system-monitor-mcp.service": "MCP servers are not staged into the image yet",
    "installd.service": "the install path is not exercised on the appliance image",
    "install-helper.service": "the install path is not exercised on the appliance image",
    "permission-helper.service": "the install path is not exercised on the appliance image",
    "xdg-desktop-portal-arlen.service": (
        "the portal is not staged into the image yet, so the security fixes it now "
        "carries are not in effect: the OpenFile descriptor TOCTOU, the resolved "
        "mount containment, the picker peer-credential check. Not urgent - nothing "
        "on the image routes through the portal yet - but this is a deferral of "
        "shipped fixes, not of scaffolding. The expensive half is the picker-ui, a "
        "Tauri app, so staging is a frontend build rather than a plain daemon step"
    ),
}


def shipped_units() -> dict[str, pathlib.Path]:
    """Every systemd unit a component ships from its own `dist/`.

    Filtered by content, not by name: the tree also holds D-Bus activation files
    called `*.service`, which live somewhere else entirely and would otherwise be
    compared against the wrong directory.
    """
    out: dict[str, pathlib.Path] = {}
    # `dist/**` rather than `dist/*`: some components file theirs under
    # `dist/systemd/`. And mkosi's build directory holds a full checkout of this
    # repo, so without excluding it the scan reads a stale copy of every unit and
    # answers about a tree nobody is editing.
    for path in ROOT.glob("**/dist/**/*.service"):
        if {"node_modules", "target", "mkosi.builddir"} & set(path.parts):
            continue
        if not re.search(r"^\[Unit\]", path.read_text(), re.M):
            continue
        out.setdefault(path.name, path)
    return out


def installed_units() -> set[str]:
    """Units the image actually places, whether by mkosi.extra or a build step."""
    placed = {
        p.name
        for sub in ("user", "system")
        for p in (EXTRA / sub).glob("*.service")
    }
    # A build step may install one itself rather than dropping it in mkosi.extra.
    scripts = "\n".join(
        p.read_text() for p in BUILD_STEPS.glob("*") if p.is_file()
    )
    return placed | {name for name in shipped_units() if name in scripts}


def main() -> int:
    shipped = shipped_units()
    if not shipped:
        sys.exit("found no shipped units; the check needs updating")
    installed = installed_units()

    problems: list[str] = []
    for name in sorted(shipped):
        if name in installed:
            if name in NOT_YET_DEPLOYED:
                problems.append(
                    f"{name} is installed but still listed as not deployed. "
                    "Delete the entry so the list keeps meaning something."
                )
            continue
        if name not in NOT_YET_DEPLOYED:
            problems.append(
                f"{name} ships a unit that the image never installs "
                f"({shipped[name].relative_to(ROOT)}). Install it, or say here why "
                "it waits - a component that is finished and undeployed fails "
                "silently, which is the failure this exists to catch."
            )

    for stale in sorted(set(NOT_YET_DEPLOYED) - set(shipped)):
        problems.append(f"{stale} is listed but ships no unit any more; delete the entry")

    if problems:
        print("shipped units and the image disagree:\n")
        for p in problems:
            print(f"  - {p}")
        return 1

    print(
        f"{len(shipped)} shipped unit(s); {len(shipped) - len(NOT_YET_DEPLOYED)} installed, "
        f"{len(NOT_YET_DEPLOYED)} deliberately waiting"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
