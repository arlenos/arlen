#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Every socket in the inventory has a decision about which app may dial it.

`arlen-run` fences a confined app's `connect()` with Landlock: the app reaches
only the sockets its profile's sections call for, and the table that says which
section opens which socket is `SOCKETS` in `daemons/arlen-run/src/sockets.rs`.
A socket missing from that table is refused to every app, which is safe and is
also how a new daemon's client would fail on the appliance with `Permission
denied` and nothing in the launcher to say why.

So the table has to name every socket the tree serves, and that list already
exists: `SERVERS` in `check-socket-servers.py`, which fails on its own when a
socket is dialled without an entry. This check ties the two together:

- a socket in the inventory with no row in the table fails,
- a row for a socket the inventory does not know fails,
- a row that opens a socket to every app fails unless the socket is in
  `EVERY_APP` below. That list is short on purpose: adding to it hands a socket
  to an app whose profile grants nothing.

    check-socket-grants.py [repo-root] [table.rs]
"""

import re
import sys
from pathlib import Path

# The sockets any confined app may dial whatever its profile says.
#
# `launch.sock`: how an app asks the shell to open a file or a link when no
# portal frontend answers. The shell decides what happens with the request.
EVERY_APP = {"launch.sock"}


def inventory(root):
    """The socket names in `SERVERS` of the socket-server gate."""
    text = (root / "dev/scripts/check-socket-servers.py").read_text()
    table = re.search(r"^SERVERS = \{(.*?)^\}", text, re.S | re.M)
    if not table:
        sys.exit("check-socket-grants: no SERVERS table in check-socket-servers.py")
    return set(re.findall(r'^\s*"([\w.-]+\.sock)":', table.group(1), re.M))


def grants(path):
    """Socket name to grant, from the `SOCKETS` table in the launcher."""
    text = path.read_text()
    table = re.search(r"pub const SOCKETS: [^=]*= &\[(.*?)^\];", text, re.S | re.M)
    if not table:
        sys.exit(f"check-socket-grants: no SOCKETS table in {path}")
    rows = re.findall(r'\(\s*"([\w.-]+)"\s*,\s*Grant::(\w+)\s*\)', table.group(1))
    if not rows:
        sys.exit(f"check-socket-grants: the SOCKETS table in {path} read as empty")
    return dict(rows)


def main():
    root = Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()
    table = Path(sys.argv[2]) if len(sys.argv) > 2 else root / "daemons/arlen-run/src/sockets.rs"
    served = inventory(root)
    granted = grants(table)

    problems = []
    for name in sorted(served - set(granted)):
        problems.append(f"{name}: in the inventory, no row in SOCKETS (no app can dial it)")
    for name in sorted(set(granted) - served):
        problems.append(f"{name}: a row in SOCKETS, not in the inventory")
    for name, grant in sorted(granted.items()):
        if grant == "Everyone" and name not in EVERY_APP:
            problems.append(f"{name}: open to every app, and not on the EVERY_APP list")
    for name in sorted(EVERY_APP):
        if granted.get(name) != "Everyone":
            problems.append(f"{name}: on the EVERY_APP list, and SOCKETS says otherwise")

    if problems:
        print("check-socket-grants: the launcher's socket table and the inventory disagree")
        for line in problems:
            print(f"  {line}")
        return 1
    print(f"check-socket-grants: OK: {len(granted)} sockets, {len(EVERY_APP)} open to every app")
    return 0


if __name__ == "__main__":
    sys.exit(main())
