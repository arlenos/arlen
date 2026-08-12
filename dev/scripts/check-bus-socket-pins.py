#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A user service that talks to the event bus must be told where the bus is.

`event-bus` is a SYSTEM service: it binds `/run/arlen/event-bus-*.sock`. A USER
service has `$XDG_RUNTIME_DIR=/run/user/<uid>`, and the SDK's fallback therefore
dials `/run/user/<uid>/arlen/…` - a path nothing ever binds. The daemon then
retries forever against a socket that will never appear, and says so only in a
log nobody was reading.

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
"""

import pathlib
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

# unit stem -> why it needs no pin despite touching the bus.
UNPINNED: dict[str, str] = {}


def source_for(binary: str) -> pathlib.Path | None:
    """The crate that builds `binary`, by its Cargo name."""
    needle = f'name = "{binary}"'
    for manifest in ROOT.rglob("Cargo.toml"):
        if {"target", "mkosi.builddir", "node_modules"} & set(manifest.parts):
            continue
        try:
            if needle in manifest.read_text(encoding="utf-8", errors="replace"):
                return manifest.parent
        except OSError:
            continue
    return None


def bus_use(crate: pathlib.Path) -> set[str]:
    """Which socket pins this crate's source actually needs."""
    needed: set[str] = set()
    src = crate / "src"
    if not src.is_dir():
        return needed
    for path in src.rglob("*.rs"):
        text = path.read_text(encoding="utf-8", errors="replace")
        if any(m in text for m in PRODUCES):
            needed.add("ARLEN_PRODUCER_SOCKET")
        if any(m in text for m in CONSUMES):
            needed.add("ARLEN_CONSUMER_SOCKET")
    return needed


def main() -> int:
    if not UNITS.is_dir():
        print(f"NOTHING WAS READ: no user units under {UNITS}", file=sys.stderr)
        return 2
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
        missing = [
            p for p in sorted(needed) if not re.search(rf"^Environment={p}=", text, re.M)
        ]
        if missing:
            problems.append(
                f"{stem} runs {binary} ({crate.relative_to(ROOT)}), whose source talks "
                f"to the event bus, and does not pin {', '.join(missing)}. A user "
                f"service without these dials /run/user/<uid>/arlen/ and retries "
                f"forever against a socket the SYSTEM bus never binds."
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
        f"{checked} user unit(s) whose daemon opens a bus socket; each pins the "
        f"direction it uses ({len(UNPINNED)} excused)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
