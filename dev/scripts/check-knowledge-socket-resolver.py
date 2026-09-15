#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A knowledge-socket client reads BOTH names, through the one resolver.

The knowledge daemon's socket answers to two environment variables:
`ARLEN_KNOWLEDGE_SOCKET`, which a session launcher exports for clients, and
`ARLEN_DAEMON_SOCKET`, which pins the daemon's own bind and is what the dev stack
sets. One socket, two variables - a wart rather than a design - and a client that
reads only one of them is broken under whichever launcher sets the other.

`os_sdk::runtime::knowledge_socket_path()` exists precisely for this and carries
the story in its own doc comment: six resolvers across the shell, Settings and
modulesd once read only `ARLEN_DAEMON_SOCKET`, fell through to the XDG default,
and every graph read in the desktop failed against a path nothing binds while the
daemon logged that it was listening.

**Those six were fixed and nineteen others were not.** On 15 September the file
manager still had twelve single-name resolvers, the text editor three, and the
terminal, the knowledge app, modulesd and the code indexer one each - every one of
them reachable from a screen. The repair had been applied to the places the boot
log named rather than to the shape, which is the third time in one night that
pattern produced a live defect.

So: nothing may call the single-name `socket_path` with a knowledge env var. The
daemon's own bind is exempt by being a different function in its own crate
(`knowledge::utils::socket_path`), which is the right place for it - a daemon
binds one path, and its unit pins it.
"""

import os
import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

# The call shape that reads one name. `os_sdk::runtime::` or a `runtime::`/bare
# import of it - all three spellings appear in the tree.
SINGLE = re.compile(
    r'(?:os_sdk::)?(?:runtime::)?socket_path\(\s*"ARLEN_(?:KNOWLEDGE|DAEMON)_SOCKET"'
)

# The daemon that OWNS the socket resolves its own bind, in its own crate helper.
# Named rather than pattern-excused: the exemption is about which crate, and a
# client that grew inside it would still be a finding worth seeing.
OWNER = "daemons/knowledge/"

SKIP_DIRS = {"target", "node_modules", ".git", ".svelte-kit", "build", "mkosi.builddir"}


def rust_files():
    for top in ("daemons", "apps", "ai", "sdk"):
        base = ROOT / top
        if not base.is_dir():
            continue
        for dirpath, dirs, files in os.walk(base):
            dirs[:] = [d for d in dirs if d not in SKIP_DIRS]
            for name in files:
                if name.endswith(".rs"):
                    yield Path(dirpath) / name


def main() -> int:
    problems = []
    read = 0
    for path in rust_files():
        read += 1
        rel = str(path.relative_to(ROOT))
        text = path.read_text(encoding="utf-8", errors="replace")
        for n, line in enumerate(text.splitlines(), 1):
            if not SINGLE.search(line):
                continue
            if rel.startswith(OWNER):
                continue
            problems.append(
                f"{rel}:{n} resolves the knowledge socket from one env name. "
                f"Use `os_sdk::runtime::knowledge_socket_path()`, which reads both, "
                f"or this client breaks under the launcher that sets the other."
            )

    if not read:
        print("!! NOTHING WAS READ: no Rust sources under daemons/, apps/, ai/ or sdk/",
              file=sys.stderr)
        return 2

    if problems:
        print("Knowledge-socket resolvers that read one name:", file=sys.stderr)
        for p in problems:
            print(f"  - {p}", file=sys.stderr)
        return 1

    print(f"{read} Rust source(s) read; every knowledge-socket client goes through the "
          f"resolver that reads both env names.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
