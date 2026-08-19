# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a component shipping a systemd unit either gets installed or says why not.

A daemon can be finished - key custody, protocol, tests, a hardened unit in its
own `dist/` - and still be absent from the image, because nothing connects the two
and nothing complains. That is not hypothetical: `arlen-ai-undo-signer` was built
and deployed nowhere while THREE producers wrote to its socket, so every
reversible action they took was journalled into nothing. The code was right and
the delivery silently was not, which is the same shape as a unit whose
ReadWritePaths omits the directory it writes, or an activation file that skips its
hardening.

It ships now, and the sequel is the reason to state the limit of this check
plainly: installed is not started. The same daemon then spent time INSTALLED,
hardened, correct and never running, because its `WantedBy=` was never turned
into a symlink and this gate does not look at enablement (12 Aug;
`check-packaged-units.sh` covers that half now). One question per check, and
knowing which question this one answers is what stops its green being read as
"the undo log works".

The gate does not demand that everything ship. Plenty of components are ahead of
the image on purpose. It demands that the answer be WRITTEN DOWN: each unit is
either installed, or listed below with the reason it waits. A silence becomes a
sentence somebody can disagree with.
"""

import pathlib
import re
import sys

# The tree to scan. An argument so this can be pointed at a fixture and shown
# to fail: a check that only ever runs against a tree that already passes
# cannot demonstrate the defect it exists for (standing rule, 11 Aug).
ROOT = (
    pathlib.Path(sys.argv[1]).resolve()
    if len(sys.argv) > 1
    else pathlib.Path(__file__).resolve().parents[2]
)
EXTRA = ROOT / "dev/mkosi/mkosi.extra/usr/lib/systemd"
BUILD_STEPS = ROOT / "dev/mkosi/mkosi.build.d"

# unit -> why it is not on the image. Each line is a CLAIM, not a shrug: if it is
# wrong, it is wrong somewhere a reader can see it. "Unreviewed" is an honest
# entry; an invented rationale is not.
# EACH ENTRY CARRIES THE DATE ITS REASON WAS LAST CHECKED, because the reason is
# a claim about the tree and claims go stale. `modulesd` sat here reading "the
# module runtime is a later phase" long after the runtime worked; it shipped on
# 15 Aug once someone read the code instead of the note.
#
# What was checked on 15 Aug for every entry below: that NO shipped component
# dials the daemon. The socket half is measured continuously by
# `check-socket-servers.py`, which is green. The bus half was read by hand, and
# exactly two shipped callers reach an unshipped daemon - Settings to
# `org.arlen.InstallDaemon1` and Files to `org.arlen.Accounts1`. Both now answer
# "unavailable on this system" rather than an empty list or a dead button, which
# is the rule for a backend that does not exist yet.
# Read by `check-admitted-ids-exist.py` as well: a daemon whose unit is deferred
# here is one whose app id no image can produce, and that gate derives the second
# fact from this list rather than keeping its own copy. Removing an entry from
# here therefore also stops excusing that daemon's allowlist entries, which is the
# behaviour you want when it finally ships.
NOT_YET_DEPLOYED: dict[str, str] = {
    "arlen-accountsd.service": (
        "online-accounts is not part of the image scope yet (15 Aug). The Files "
        "sidebar is the one shipped caller and distinguishes absent from "
        "none-configured, with a test for it"
    ),
    "arlen-connectionsd.service": "connections daemon is not part of the image scope yet (15 Aug, no shipped caller)",
    "arlen-transferd.service": "transfer daemon is not part of the image scope yet (15 Aug, no shipped caller)",
    "arlen-settings-broker.service": "settings broker is not part of the image scope yet (15 Aug, no shipped caller)",
    "arlen-trash-cleanup.service": "trash retention timer, unreviewed for deployment (15 Aug)",
    "arlen-file-manager-mcp.service": "MCP servers are not staged into the image yet (15 Aug)",
    "arlen-knowledge-mcp.service": "MCP servers are not staged into the image yet (15 Aug)",
    "arlen-system-monitor-mcp.service": "MCP servers are not staged into the image yet (15 Aug)",
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


def installs_unit(script: str, unit: str) -> bool:
    """Does this build-step text actually PLACE `unit`, rather than mention it?

    Split out so it can be tested on its own: the gate around it reads the real
    tree and carries hand-kept lists, so a synthetic fixture cannot exercise it
    without dragging those along.

    A line counts when it copies or installs. Comments never count, which is the
    whole point - see `installed_units`.
    """
    for line in script.splitlines():
        stripped = line.lstrip()
        if stripped.startswith("#"):
            continue
        if ("install " in stripped or stripped.startswith("cp ")) and unit in stripped:
            return True
    return False


def installed_units() -> set[str]:
    """Units the image actually places, whether by mkosi.extra or a build step."""
    placed = {
        p.name
        for sub in ("user", "system")
        for p in (EXTRA / sub).glob("*.service")
    }
    # A build step may install one itself rather than dropping it in mkosi.extra.
    #
    # PER LINE, AND ONLY LINES THAT PLACE A FILE. This used to substring-match the
    # whole script text, which meant a unit MENTIONED anywhere counted as
    # installed - including in a comment. On 19 Aug the install-path phase said in
    # prose that it deliberately does NOT ship `arlen-trash-cleanup.service`, and
    # the gate read its own name in that sentence and reported the unit as
    # deployed. Documenting a deliberate omission is exactly the behaviour this
    # file asks for elsewhere, so the checker had to stop punishing it.
    #
    # A line counts when it copies or installs: `install -D...`, `cp ...`. That is
    # narrow enough to ignore prose and wide enough for how these phases are
    # written; a phase that placed a unit some other way would be missed, and the
    # honest cost of the narrower rule is that it can now under-report rather than
    # over-report. Under-reporting fails LOUDLY here - the unit stays listed as
    # waiting while it is on the image - which is the direction to err in.
    texts = [p.read_text() for p in BUILD_STEPS.glob("*") if p.is_file()]
    return placed | {n for n in shipped_units() if any(installs_unit(t, n) for t in texts)}



# The second half of this gate, and the reason it exists: the first half can only
# ever check components that HAVE a unit.
#
# `kernel-layer` is the case that showed it. It is not on the image - no unit, no
# build phase - and it produces `file.opened`, which the whole File-and-timeline
# half of the graph is built from. The gate designed to catch exactly that could
# not see it, because a component nobody wrote a `dist/` file for is not a subject.
# `modulesd` is absent WITH a written reason precisely because someone did write
# one; the difference between the two was never deliberateness.
#
# So the subjects here are DERIVED: every crate under `daemons/`. Each must either
# have a unit somewhere in its own tree, or say below why it has none. A
# hand-maintained list is strongest exactly where it is needed least.
NO_UNIT: dict[str, str] = {
    "calendar": (
        "no process yet: this is the store half, the part that reads the "
        "calendar directory and says what it holds. `calendar-app.md` section 2 "
        "puts it outside the app because reminders must outlive the window, and "
        "the unit lands with the process that serves `org.arlen.Calendar1` and "
        "registers triggers with `org.arlen.Clock1`"
    ),
    "arlen-run": (
        "no unit by design: it is a fork-exec launcher a session invokes per app, "
        "not a service that runs"
    ),
    "session": (
        "no unit by design, like arlen-run: greetd starts the session from "
        "`[initial_session]`, it is not a service that runs. The crate holds the "
        "compiled session's decisions while the shipped login path is still the "
        "shell script at /usr/bin/arlen-session"
    ),
    "session-supervisor": (
        "the decision half is built and tested; the systemd and broker seams are "
        "not wired, so its binary exits rather than pretending to supervise. A "
        "unit now would start something that immediately fails, which is the "
        "silent-success shape this work exists to remove. The unit lands with the "
        "seams, and `arlen-session` launches it the way it launches the shell"
    ),
    "bridge-ingest": "runs from the dogfood path today, unreviewed as a standalone service",
    "integration-packages": "a library and CLI for package assembly, not a running service",
    "lock-auth": "the lock screen's auth backend, consumed in-process, not a service",
    # Corrected 18 Aug when the portal was staged: the old reason ("the portal is
    # itself unstaged") stopped being true that day. What actually keeps print
    # waiting is the other half - five built operations that no surface reaches.
    "print": "the print operations are built but no surface calls them (18 Aug)",
    "sentinel-detect": "detection library, no daemon shape yet",
}


def crates_without_units() -> list[str]:
    """Daemon crates that ship no unit anywhere in their own tree.

    Looks beyond `dist/`: the knowledge daemon files its unit under `systemd/`, so
    a dist-only scan reports the tree's most load-bearing daemon as unit-less.
    """
    missing = []
    for crate in sorted((ROOT / "daemons").iterdir()):
        if not (crate / "Cargo.toml").is_file():
            continue
        units = [
            u
            for u in crate.glob("**/*.service")
            if not {"target", "node_modules"} & set(u.parts)
        ]
        if not units:
            missing.append(crate.name)
    return missing

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

    # The derived half: a crate with no unit at all is a subject too.
    unit_less = crates_without_units()
    for crate in unit_less:
        if crate not in NO_UNIT:
            problems.append(
                f"daemons/{crate} ships no systemd unit anywhere and says nothing "
                "about why. Write the unit, or add the reason to NO_UNIT - a "
                "component nobody listed is exactly what this half of the gate "
                "exists to notice"
            )
    for stale in sorted(set(NO_UNIT) - set(unit_less)):
        problems.append(
            f"daemons/{stale} is excused for having no unit and now has one; "
            "delete the entry so the excuse cannot outlive its subject"
        )

    if problems:
        print("shipped units and the image disagree:\n")
        for p in problems:
            print(f"  - {p}")
        return 1

    print(
        f"{len(shipped)} shipped unit(s); {len(shipped) - len(NOT_YET_DEPLOYED)} installed, "
        f"{len(NOT_YET_DEPLOYED)} deliberately waiting; "
        f"{len(unit_less)} daemon crate(s) carry no unit, each with a stated reason"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
