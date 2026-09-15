#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A user service that talks to a per-user daemon must not pin the system path.

INVERTED ON 15 AUG, when the bus moved per-user. What follows is the defect it was
written for, kept because the shape is the same and only the direction changed.

`event-bus` WAS a system service binding `/run/arlen/event-bus-*.sock`, while a
user service has `$XDG_RUNTIME_DIR=/run/user/<uid>` and the SDK's fallback dialled
`/run/user/<uid>/arlen/…` - a path nothing bound. The daemon retried forever
against a socket that would never appear, and said so only in a log nobody read.
The repair was an `Environment=` pin in each user unit.

The bus binds per-user now, so the SDK default is right and the pins are what
would be wrong: a unit still naming `/run/arlen` dials the path nothing binds. So
this checks that no unit pins the system path, and the eighteen that carried one
had it removed in the same change.

Measured on the 12 Aug boot: `arlen-anomalyd` logged `event bus subscribe failed`
every five seconds for the whole run - the anomaly detector is the channel the
interaction model reserves for warning the user, and it had never subscribed -
while `arlen-powerd` published `power.state` into the same non-existent path.

**The reason this needs a check rather than a fix is where the previous fix went.**
`arlen-code-indexer.service` already carried the two `Environment=` lines, with a
comment saying they were added "where a boot log showed it was missing". The
defect had been found, understood and repaired at the SYMPTOM rather than at the
SHAPE, so three siblings kept it until another boot happened to expose one.

So: every user unit whose daemon source opens a bus socket must pin THAT socket -
the producer if it publishes, the consumer if it subscribes, both if both - or say
why not. The unit-to-source mapping is derived from the
ExecStart basename against the Cargo package that builds it, so a renamed daemon
does not quietly fall out of the check.

WIDENED 15 SEP TO THE KNOWLEDGE SOCKET, because the check was written for the
shape and then applied to one socket. `arlen-graph.service` binds
`/run/user/%U/arlen/knowledge.sock` and four user units pinned
`/run/arlen/knowledge.sock`: the AI engine, the notification daemon, the consent
broker and the code indexer, in both their image copy and their `dist/` canonical.
Every one of them dialled a path nothing binds.

The code indexer is the one that shows why a check beats a fix. Its unit carries a
comment saying, in as many words, that the socket pins were removed on 15 August
because the bus went per-user and "keeping them now would recreate that defect
pointing the other way" - and the line directly beneath that comment pinned the
knowledge socket at `/run/arlen`. The person who removed the bus pins was looking
at the bus. Three siblings had the same line and nothing was looking at any of
them.
"""

import pathlib
import os
import re
import sys

ROOT = (
    pathlib.Path(sys.argv[1]).resolve()
    if len(sys.argv) > 1
    else pathlib.Path(__file__).resolve().parents[2]
)
UNITS = ROOT / "dev/mkosi/mkosi.extra/usr/lib/systemd/user"
PINS = ("ARLEN_PRODUCER_SOCKET", "ARLEN_CONSUMER_SOCKET")

# Per DIRECTION, because they are separate needs: a daemon that only publishes has
# no use for the consumer socket, and demanding both would teach people to add a
# line that means nothing. The first draft did exactly that and reported
# `journald-parser` - a pure producer - for missing a consumer pin.
#
# `event_bus` as a bare token is NOT a marker either: `ai-undo-signer` has a struct
# field of that name and touches no bus at all. A marker has to name the thing that
# opens the socket.
PRODUCES = ("ARLEN_PRODUCER_SOCKET", "UnixEventEmitter", "EventEmitter")
CONSUMES = ("ARLEN_CONSUMER_SOCKET", "EventConsumer", "consumer_socket")

# The knowledge daemon's read socket, the same shape one daemon over. Two env
# names reach it - the daemon's own bind variable and the client one - and both
# have to be checked, because `ai-engine-daemon` pinned BOTH at the system path.
READS_GRAPH = (
    "ARLEN_KNOWLEDGE_SOCKET",
    "ARLEN_DAEMON_SOCKET",
    "UnixGraphClient",
    "UnixGraph::",
)
GRAPH_PINS = ("ARLEN_KNOWLEDGE_SOCKET", "ARLEN_DAEMON_SOCKET")

# unit stem -> why it needs no pin despite touching the bus.
UNPINNED: dict[str, str] = {}


#: Cargo name -> the directory of the manifest that declares it, built once.
_MANIFESTS: dict[str, pathlib.Path] | None = None

#: Directories a source walk must not descend into. Pruned rather than filtered
#: after the fact: `rglob` walks a `target/` in full and only then discards what
#: it found there, which is where this check spent most of its 29 seconds.
SKIP_DIRS = {"target", "mkosi.builddir", "node_modules", ".git", ".svelte-kit", "build"}


def manifests() -> dict[str, pathlib.Path]:
    """Every crate in the tree by the name it publishes, read once.

    This used to be a full-tree walk PER BINARY - one `rglob("Cargo.toml")` for
    each unit's ExecStart, each of them descending into every build directory in
    the repo. Twenty-odd walks of a tree whose build output dwarfs its source.
    """
    global _MANIFESTS
    if _MANIFESTS is not None:
        return _MANIFESTS
    found: dict[str, pathlib.Path] = {}
    for base, dirs, files in os.walk(ROOT):
        dirs[:] = [d for d in dirs if d not in SKIP_DIRS]
        if "Cargo.toml" not in files:
            continue
        path = pathlib.Path(base) / "Cargo.toml"
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        for m in re.finditer(r'^name\s*=\s*"([^"]+)"', text, re.M):
            found.setdefault(m.group(1), path.parent)
    _MANIFESTS = found
    return found


def source_for(binary: str) -> pathlib.Path | None:
    """The crate that builds `binary`, by its Cargo name."""
    return manifests().get(binary)


def bus_use(crate: pathlib.Path) -> set[str]:
    """Which socket pins this crate's source actually needs."""
    needed: set[str] = set()
    src = crate / "src"
    if not src.is_dir():
        return needed
    for base, dirs, files in os.walk(src):
        dirs[:] = [d for d in dirs if d not in SKIP_DIRS]
        for name in files:
            if not name.endswith(".rs"):
                continue
            text = (pathlib.Path(base) / name).read_text(
                encoding="utf-8", errors="replace"
            )
            if any(m in text for m in PRODUCES):
                needed.add("ARLEN_PRODUCER_SOCKET")
            if any(m in text for m in CONSUMES):
                needed.add("ARLEN_CONSUMER_SOCKET")
            if any(m in text for m in READS_GRAPH):
                needed.update(GRAPH_PINS)
    return needed


def main() -> int:
    if not UNITS.is_dir():
        print(f"NOTHING WAS READ: no user units under {UNITS}", file=sys.stderr)
        return 2
    # The image units only. A `dist/` canonical that drifts from its packaged
    # copy is `check-packaged-units.sh`'s question and it compares them byte for
    # byte, so reading both trees here would report the same line twice and drag
    # in dist units the image does not ship (a `busctl` ExecStart, the permission
    # helper) as unresolved binaries.
    units = sorted(UNITS.glob("*.service"))
    if not units:
        print(f"NOTHING WAS READ: no user units in {UNITS}", file=sys.stderr)
        return 2

    problems: list[str] = []
    checked = 0
    unresolved: list[str] = []
    for unit in units:
        text = unit.read_text()
        m = re.search(r"^ExecStart=(\S+)", text, re.M)
        if not m:
            continue
        binary = m.group(1).rsplit("/", 1)[-1]
        crate = source_for(binary)
        if crate is None:
            unresolved.append(binary)
            continue
        needed = bus_use(crate)
        if not needed:
            continue
        checked += 1
        stem = unit.stem
        if stem in UNPINNED:
            continue
        pinned = [
            p
            for p in sorted(needed)
            if re.search(rf"^Environment={p}=/run/arlen/", text, re.M)
        ]
        if pinned:
            problems.append(
                f"{stem} runs {binary} ({crate.relative_to(ROOT)}), whose source opens "
                f"a per-user socket, and pins {', '.join(pinned)} at /run/arlen. Those "
                f"daemons bind under /run/user/%U/arlen now, so that path is the one "
                f"nothing binds - this unit would dial into nothing and retry forever, "
                f"which is the same defect this check was written for, pointing the "
                f"other way."
            )

    # A binary whose crate cannot be found is not a pass: it is a unit this check
    # silently skipped, which is the shape it exists to refuse.
    for binary in unresolved:
        problems.append(
            f"{binary} is started by a user unit and no Cargo package builds a "
            f"binary of that name. Either the unit points at something that does not "
            f"exist, or the mapping this check derives is broken - both mean it is "
            f"not checking that unit."
        )

    for stale in sorted(UNPINNED):
        if not (UNITS / f"{stale}.service").is_file():
            problems.append(f"{stale} is excused here and ships no unit; delete the entry")

    if problems:
        print("event-bus socket pins:\n")
        for p in problems:
            print(f"  - {p}")
        return 1

    print(
        f"{checked} user unit(s) whose daemon opens a per-user socket; none pins the "
        f"system path those daemons moved away from ({len(UNPINNED)} excused)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
