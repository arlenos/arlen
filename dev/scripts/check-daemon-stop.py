#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Check that a daemon shipping a long-running systemd unit handles SIGTERM.

WHY. `systemctl stop`, `systemctl restart` and every reboot send SIGTERM. A daemon that does
not catch it dies on the default disposition, wherever it happened to be - and whatever
shutdown path it wrote for `ctrl_c` never runs, because a service is not stopped by Ctrl-C.

That is not theoretical here. The knowledge daemon had no handler at all, so its ladybug
database was never closed on a stop; a store taken down uncleanly refused to reopen, and the
whole knowledge graph answered "ladybug thread has stopped" from then on. Fixed on 16 August,
and the survey that followed found eight more daemons awaiting only `ctrl_c` - including
`online-accounts`, which holds the AEAD token vault, and `transfer-daemon`, which keeps a
dual-ledger audit. Both are exactly the state a hard stop mid-write can leave inconsistent.
This check then found two the survey could not: `event-bus` and `wallpaper` handle no signal at
all, so a grep that looked for daemons WITH a handler never saw them - which is the argument
for a check over a reading.

WHAT COUNTS. Only a long-running systemd service: a `[Service]` unit with an `ExecStart`,
excluding `Type=oneshot` (which is supposed to run and exit) and excluding the `org.*.service`
D-Bus ACTIVATION files, which are a different format that only names a bus and a binary.

THE BASELINE is the honest part. Ten daemons predate this rule, and turning it on red would
just add a permanently failing check nobody reads - the habit that costs a real failure later.
So they are listed here with what each one holds, the check fails for anything NEW, and the
list is meant to shrink. Removing a name is the fix; adding one needs a reason in writing.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = (
    Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
)
DAEMONS = ROOT / "daemons"

# Daemons that ship a unit and do not yet handle SIGTERM, with what a hard stop
# risks for each. Shrink this list; do not grow it.
BASELINE = {
    "clock": "clock state only, a hard stop costs nothing recorded",
    "event-bus": "routes only - socket writes, no persistent store, so a hard stop "
    "loses in-flight events exactly as any crash does",
    "file-manager-mcp": "stateless bridge",
    "knowledge-mcp": "stateless bridge",
    "notification-daemon": "SQLite, committed per notification",
    "online-accounts": "HOLDS THE AEAD TOKEN VAULT - the one worth fixing first",
    "system-monitor-mcp": "stateless bridge",
    "terminal-run-mcp": "stateless bridge",
    "transfer-daemon": "KEEPS A DUAL-LEDGER AUDIT - fix alongside online-accounts",
    "wallpaper": "reads the manifest and paints; the settings app owns the writes",
}

EXEC_START = re.compile(r"^ExecStart=", re.M)
SERVICE_SECTION = re.compile(r"^\[Service\]", re.M)
ONESHOT = re.compile(r"^Type=oneshot", re.M)


def long_running_units(daemon: Path) -> list[Path]:
    """The daemon's own systemd services, excluding oneshots and D-Bus activation."""
    out = []
    for unit in sorted((daemon / "dist").glob("*.service")):
        text = unit.read_text(encoding="utf-8", errors="replace")
        if not SERVICE_SECTION.search(text) or not EXEC_START.search(text):
            continue  # a D-Bus activation file, not a systemd service
        if ONESHOT.search(text):
            continue  # runs and exits by design
        out.append(unit)
    return out


def handles_sigterm(daemon: Path) -> bool:
    src = daemon / "src"
    if not src.is_dir():
        return True  # nothing to judge
    return any(
        "SignalKind::terminate" in p.read_text(encoding="utf-8", errors="replace")
        for p in src.rglob("*.rs")
    )


def main() -> int:
    if not DAEMONS.is_dir():
        print("daemons/ is missing; the layout moved and this check did not")
        return 1

    missing: list[str] = []
    checked = 0
    # Baseline names that are PRESENT in this tree and now handle SIGTERM. Only
    # those can be called stale - a tree that simply does not contain a daemon
    # says nothing about whether its entry is still needed, and treating absence
    # as fixed made this check fail on every fixture that is not the whole repo.
    stale: list[str] = []

    for daemon in sorted(p for p in DAEMONS.iterdir() if p.is_dir()):
        units = long_running_units(daemon)
        if not units:
            continue
        checked += 1
        if handles_sigterm(daemon):
            if daemon.name in BASELINE:
                stale.append(daemon.name)
            continue
        if daemon.name in BASELINE:
            continue
        missing.append(
            f"{daemon.name}: ships {units[0].relative_to(ROOT)} but no source handles "
            f"SIGTERM - `systemctl stop` will kill it wherever it stands"
        )

    if not checked:
        print("no long-running daemon units found; that is not plausible")
        return 1

    if missing:
        print("\ndaemons that ship a service but never see a stop:\n")
        for m in missing:
            print(f"  - {m}")
        print(
            "\nHandle SIGTERM (see daemons/knowledge/src/main.rs: close what must be closed, "
            "await it, then leave). If a hard stop is genuinely safe, add the daemon to "
            "BASELINE with the reason."
        )
        return 1

    if stale:
        print(
            "these are in BASELINE but now handle SIGTERM; remove them from the list: "
            + ", ".join(sorted(stale))
        )
        return 1

    print(f"{checked} daemon(s) with a long-running unit: {len(BASELINE)} on the baseline, no new ones.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
