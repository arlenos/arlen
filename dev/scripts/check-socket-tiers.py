#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a socket path falls through the per-user tier, not straight to /run.

An Arlen socket answers to three tiers, in order: an `ARLEN_*_SOCKET` pin, then
`$XDG_RUNTIME_DIR/arlen/<name>`, then `/run/arlen/<name>`. The middle one is the
one that matters, because the buses and daemons became user services and bind
there, and `arlen-session` deliberately pins nothing - a test in `daemons/session`
asserts it does not, since a pin here would override the per-user resolution for
every process in the session.

A resolver that reads the pin and then falls straight to `/run/arlen` therefore
dials a path nothing binds on a booted machine, while its own logs say it is
connecting. It has happened three times:

  * **installd**, whose own comment records it: "an unpinned installd dialled
    nothing".
  * **six resolvers across the shell, Settings and modulesd**, reading one of the
    knowledge socket's two env names, so every graph read in the desktop failed
    while the daemon logged that it was listening.
  * **`sdk/tauri-plugin-shell`**, which is the one this check was written for. It
    is loaded INSIDE every Arlen app, so its three hardcoded fallbacks took the
    whole app side of the bus with them: publishing a menu, receiving the click
    back, the toolbar, the shortcut relay and the graph read. The same crate's
    `theme.rs` used the shared resolver, which is what made it an oversight
    rather than a decision.

WHAT IS CHECKED. Every function holding a `/run/arlen/<something>.sock` literal
that also reads an `ARLEN_*` environment variable. That pairing is a resolver by
construction: something that consults a pin and has a system fallback. Such a
function must also name `XDG_RUNTIME_DIR`.

A function with the literal and NO env read is not a resolver - it is a system
service naming its own socket, or a doc example - and is left alone. Delegating
to `os_sdk::runtime::socket_path` never trips this: those functions carry no
literal at all, which is the point of having one resolver.
"""

import re
import sys
from pathlib import Path

RUN_LITERAL = re.compile(r'"/run/arlen/[^"]*\.sock"')
ENV_READ = re.compile(r'"ARLEN_[A-Z_]*"')
FN_START = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const\s+|async\s+)*fn\s")
SKIP = ("/target/", "/node_modules/")

# Resolvers knowingly missing the tier, with why. Empty, and that is the point.
CARRIED: dict[str, str] = {}


def bodies(text: str) -> list[tuple[int, str]]:
    """Every function body in `text`, as (1-based start line, source)."""
    lines = text.splitlines()
    starts = [i for i, line in enumerate(lines) if FN_START.match(line)]
    out = []
    for n, start in enumerate(starts):
        end = starts[n + 1] if n + 1 < len(starts) else len(lines)
        out.append((start + 1, "\n".join(lines[start:end])))
    return out


def undecorated(source: str) -> str:
    """The source with doc comments and line comments dropped.

    A comment naming `/run/arlen/…` is prose about the fallback, not the
    fallback; several of the correct resolvers explain themselves that way and
    would otherwise read as the thing they are warning about.
    """
    return "\n".join(
        line for line in source.splitlines() if not line.lstrip().startswith("//")
    )


def main() -> int:
    root = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
    problems: list[str] = []
    checked = 0
    read = 0

    for path in sorted(root.rglob("*.rs")):
        if any(s in str(path) for s in SKIP):
            continue
        read += 1
        try:
            text = path.read_text(errors="replace")
        except OSError:
            continue
        if "/run/arlen/" not in text:
            continue
        # Tests pin literal paths on purpose; they are asserting the fallback.
        head = text.split("#[cfg(test)]")[0]
        rel = path.relative_to(root)
        for line_no, body in bodies(head):
            source = undecorated(body)
            if not RUN_LITERAL.search(source) or not ENV_READ.search(source):
                continue
            checked += 1
            key = f"{rel}:{line_no}"
            if key in CARRIED:
                continue
            if "XDG_RUNTIME_DIR" not in source:
                problems.append(
                    f"  - {key}: reads an ARLEN_* pin and falls straight to "
                    f"/run/arlen, skipping $XDG_RUNTIME_DIR/arlen where the "
                    f"daemons actually bind"
                )

    if problems:
        print("Socket resolvers that skip the per-user tier:")
        print()
        print("\n".join(problems))
        print()
        print("  On a booted session this dials a path nothing binds, while the log")
        print("  says it is connecting. Use os_sdk::runtime::socket_path, or add the")
        print("  $XDG_RUNTIME_DIR/arlen tier between the pin and /run/arlen.")
        return 1

    # Keyed on Rust files read, not resolvers found: a tree with sources and no
    # socket resolver in them is a real tree with nothing here to judge, and
    # four of this check's own control cases are exactly that. A tree with no
    # Rust in it at all is a walk that reached nothing.
    if read == 0:
        print("check-socket-tiers: no Rust source found, so the scan is pointed wrong")
        return 1

    print(f"check-socket-tiers: {checked} socket resolver(s), each with the per-user tier")
    return 0


if __name__ == "__main__":
    sys.exit(main())
