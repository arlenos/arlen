#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A portal interface the daemon serves must be one the frontend knows to route.

An impl backend is reached for exactly the interfaces it ADVERTISES. Two files do
that advertising - `portals/arlen.portal` says what this backend can serve, and
`arlen-portals.conf` says which backend the frontend should prefer - and the conf
file's own comment states the consequence of forgetting one: "one implemented but
not named here is simply never routed to us."

That is not hypothetical. On 19 August the daemon had served
`org.freedesktop.impl.portal.Print` since it was written, both files listed
FileChooser, OpenURI and Screenshot, and so printing from an app could never have
reached our backend on the image. The code was live on the bus and unreachable,
which reads in a review as a finished feature.

The reverse is the worse failure and is checked too. An interface named in either
file but not served is a request the frontend routes to a backend that answers
`UnknownMethod`, which the user meets as a dialog that opens and fails - the
`.portal` file says so itself: "an interface listed but unserviceable is not a
degraded feature - it is a file dialog that opens and fails, which reads to a user
as a broken app rather than a missing portal."

An interface may be served and deliberately unadvertised, which is why WAITING
exists. Each entry says what it waits FOR, because "not yet" and "we decided not
to" are different states and only the reason tells them apart.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ARGS = [a for a in sys.argv[1:]]
ROOT = Path(ARGS[0]).resolve() if ARGS else Path(__file__).resolve().parents[2]

DAEMON = ROOT / "daemons/xdg-portal/daemon/src"
DIST = ROOT / "daemons/xdg-portal/dist/xdg-desktop-portal"
PORTAL_FILE = DIST / "portals/arlen.portal"
PREFERRED_FILE = DIST / "arlen-portals.conf"

# Served on the bus, deliberately not advertised, and what each waits for. Checked
# 19 August rather than assumed.
WAITING: dict[str, str] = {
    "org.freedesktop.impl.portal.ScreenCast": (
        "the whole screencast group waits on PipeWire; advertising it would route "
        "screen-sharing requests to a backend that cannot produce a stream"
    ),
    "org.freedesktop.impl.portal.Print": (
        "the image stages no spooler - `daemons/print/src/cups.rs` speaks IPP to "
        "localhost:631 and `cups` is in neither mkosi.conf nor the build steps - and "
        "whether Arlen ships CUPS or prints direct to an IPP-Everywhere printer is an "
        "open decision. Advertising it first would ship a print that silently fails"
    ),
}


def interface_names() -> dict[str, str]:
    """Rust type to the D-Bus interface it implements, from the zbus attribute."""
    found: dict[str, str] = {}
    for path in sorted((DAEMON / "interfaces").glob("*.rs")):
        text = path.read_text(encoding="utf-8", errors="replace")
        # The `impl` is not always the next thing after the attribute: two of the
        # five carry a clippy allow, with a trailing comment, in between. Matching
        # the two as adjacent found three interfaces of five and called the other
        # two unserved - the same "it matches the shape I happened to write"
        # failure this check exists to catch, one level up. So: find the
        # attribute, then take the next `impl` after it.
        for m in re.finditer(r'#\[interface\(\s*name\s*=\s*"([^"]+)"', text):
            after = re.search(r"\bimpl\s+(\w+)", text[m.end() :])
            if after:
                found[after.group(1)] = m.group(1)
    return found


def served(types: dict[str, str]) -> set[str]:
    """The interfaces main.rs actually puts on the bus.

    Keyed off `serve_at`, because a type that is imported and constructed but never
    served is not on the bus - and that difference is the entire subject here.
    """
    main = (DAEMON / "main.rs").read_text(encoding="utf-8", errors="replace")
    out = set()
    for m in re.finditer(r"serve_at\(\s*[A-Za-z_]+\s*,\s*(\w+)::", main):
        name = types.get(m.group(1))
        if name:
            out.add(name)
    return out


def advertised() -> tuple[set[str], set[str]]:
    """What each of the two files names."""
    portal = PORTAL_FILE.read_text(encoding="utf-8", errors="replace")
    line = ""
    for raw in portal.splitlines():
        if raw.strip().startswith("Interfaces="):
            line = raw.split("=", 1)[1]
            break
    in_portal = {p.strip() for p in line.split(";") if p.strip()}

    conf = PREFERRED_FILE.read_text(encoding="utf-8", errors="replace")
    in_conf = set()
    for raw in conf.splitlines():
        raw = raw.strip()
        if raw.startswith("#") or "=" not in raw or raw.startswith("["):
            continue
        in_conf.add(raw.split("=", 1)[0].strip())
    return in_portal, in_conf


def main() -> int:
    for path in (DAEMON / "main.rs", PORTAL_FILE, PREFERRED_FILE):
        if not path.is_file():
            print(f"check-portal-interfaces: {path} is missing")
            return 1

    types = interface_names()
    on_bus = served(types)
    in_portal, in_conf = advertised()
    problems: list[str] = []

    for name in sorted(on_bus):
        missing = [
            where
            for where, names in (("portals/arlen.portal", in_portal), ("arlen-portals.conf", in_conf))
            if name not in names
        ]
        # Half-advertised is always wrong, waiting or not, and is checked FIRST:
        # the two files disagreeing means one of them is a lie about the same
        # interface, and reporting it as "not advertised at all" would send a
        # reader to add an entry that is already there.
        if len(missing) == 1:
            problems.append(
                f"{name} is named in one advertising file and not the other (missing from "
                f"{missing[0]}). The two must agree; a `.portal` entry with no preference "
                "rests on the deprecated `UseIn=` key, and a preference with no `.portal` "
                "entry names a backend the frontend cannot confirm."
            )
        elif missing and name not in WAITING:
            problems.append(
                f"{name} is served by the daemon but not named in {' or '.join(missing)}, "
                "so the frontend will never route it here. Add it to BOTH files, or to "
                "WAITING with what it waits for."
            )

    for name in sorted(in_portal | in_conf):
        if name not in on_bus:
            problems.append(
                f"{name} is advertised but the daemon does not serve it, so the frontend "
                "routes a request to a backend that cannot answer. Serve it or drop it "
                "from both files."
            )

    for name in sorted(WAITING):
        if name not in on_bus:
            problems.append(
                f"{name} is in WAITING but is not served at all, so the entry is stale."
            )

    if problems:
        print("check-portal-interfaces: the bus and the advertising disagree\n")
        for p in problems:
            print(f"  {p}")
        return 1

    waiting = ", ".join(sorted(n.rsplit(".", 1)[-1] for n in WAITING))
    print(
        f"check-portal-interfaces: {len(on_bus - set(WAITING))} interfaces served and advertised"
        + (f"; {waiting} served and waiting" if waiting else "")
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
