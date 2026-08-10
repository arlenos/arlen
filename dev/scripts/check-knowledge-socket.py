#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A client that resolves the knowledge socket must read the name a launcher sets.

One socket, two environment variables. `ARLEN_DAEMON_SOCKET` pins where the daemon
BINDS and is what `start-dev.sh` sets; `ARLEN_KNOWLEDGE_SOCKET` is what a session
launcher exports for CLIENTS, and it is what `arlen-session` exports on a booted
image. A client that reads only the first falls through to
`$XDG_RUNTIME_DIR/arlen/knowledge.sock`, which nothing binds.

That is not a hypothetical shape. On the night of 9 August, seven resolvers
implemented this rule and five had it wrong, which meant: every graph read in the
desktop shell dead (Projects, Focus Mode, recent files, Waypointer search), two
daemons writing to the graph through a path nothing binds, and Settings' About
page reporting the knowledge daemon as not running on a system where it was. Each
of the five carried a comment saying it matched the others.

The rule, deliberately narrow so it does not become a list of exceptions:

    a Rust file that names `knowledge.sock` AND EITHER builds a path from
    `XDG_RUNTIME_DIR` OR asks the SDK to resolve one under the bind variable
    is a client-side resolver, and must mention `ARLEN_KNOWLEDGE_SOCKET` or
    call `knowledge_socket_path`.

The `knowledge.sock` half stops it flagging every socket in the tree; the second
half stops it flagging every file that merely names the path in a doc comment or
a default constant.

The SDK clause was added after checking a claim rather than after a failure. The
`apps/harness` exclusion below was written on the belief that harness had the
broken shape - it does, as `socket_path("ARLEN_DAEMON_SOCKET", "knowledge.sock")`
- but the exclusion was doing nothing, because harness never names
`XDG_RUNTIME_DIR` itself: the fallback lives inside the helper it calls. So the
first version of this gate was blind to the exact spelling that two of the seven
resolvers used. An exclusion that turns out to be inert is worth reading as a
signal about the rule, not just tidied away.

NOT covered: whether the resolution ORDER is right (a file could name the variable
and rank it last), and whether a caller uses the resolver it has. Both need more
than a grep, and this catches the shape that actually shipped.

Shown to fail before being trusted: `dev/scripts/test-check-knowledge-socket.mjs`.

Usage: check-knowledge-socket.py [repo-root]
"""

import sys
from pathlib import Path

# Files that match the shape above but are not client resolvers. Each entry
# says why, because an exclusion without a reason is how a gate rots into a list of
# things someone once silenced.
EXCLUDED = {
    "daemons/knowledge": "the daemon that BINDS the socket; ARLEN_DAEMON_SOCKET is the bind variable and is correct here",
    "dev/integration": "the test harness pins both variables itself, and asserting on its own fixture proves nothing",
    "apps/harness": "arlen-ui's live work; the shape is present and is reported in coder-reports, not fixed here",
    "dev/mkosi/mkosi.builddir": "the build cache, a copy of the tree at some earlier commit",
    "target": "build output",
}

CLIENT_NAME = "ARLEN_KNOWLEDGE_SOCKET"
SDK_HELPER = "knowledge_socket_path"


def excluded_for(rel: str):
    """The reason `rel` is out of scope, or None if it is in scope."""
    for prefix, reason in EXCLUDED.items():
        if rel == prefix or rel.startswith(prefix + "/"):
            return reason
    return None


def offenders(root: Path):
    """Every in-scope client resolver that never mentions the client variable."""
    found = []
    for path in sorted(root.rglob("*.rs")):
        rel = str(path.relative_to(root))
        if excluded_for(rel):
            continue
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        if "knowledge.sock" not in text:
            continue
        builds_from_xdg = "XDG_RUNTIME_DIR" in text
        asks_sdk_under_bind_name = 'socket_path("ARLEN_DAEMON_SOCKET"' in text
        if not (builds_from_xdg or asks_sdk_under_bind_name):
            continue
        if CLIENT_NAME in text or SDK_HELPER in text:
            continue
        found.append(rel)
    return found


def main() -> int:
    root = Path(sys.argv[1] if len(sys.argv) > 1 else Path(__file__).resolve().parents[2])
    found = offenders(root)
    if not found:
        print(
            "knowledge socket: every client resolver reads the name a launcher sets"
        )
        return 0
    print(
        f"CLIENT RESOLVER IGNORES {CLIENT_NAME}: {len(found)} file(s)",
        file=sys.stderr,
    )
    for rel in found:
        print(f"  {rel}", file=sys.stderr)
    print(
        f"  Each resolves a knowledge-socket path - from XDG_RUNTIME_DIR directly, or\n"
        f"  by asking the SDK under the bind variable - without ever consulting\n"
        f"  {CLIENT_NAME}, which is the name `arlen-session` exports. On a\n"
        f"  booted image that resolves to a path nothing binds, and the failure is\n"
        f"  silent: the client reports the daemon as absent while it is listening.\n"
        f"  Read both names, or call os_sdk::runtime::{SDK_HELPER}().",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
