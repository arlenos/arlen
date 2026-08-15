#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""No component writes the event bus's socket path down; it asks the SDK.

WHY THIS EXISTS. The bus moved per-user on 15 Aug, and the work looked done: every
`Environment=` pin naming `/run/arlen` came out of eighteen units, and four gates
went green over it. The boot said otherwise - the shell logged `Event Bus consumer
cannot reach /run/arlen/event-bus-consumer.sock` and the graph ingested nothing.

Eight call sites across five apps had the path written into the SOURCE, so no
amount of fixing units could reach them. That is the gap this closes: a unit-level
check cannot see a string constant, and a string constant outlives every
deployment change made around it.

The SDK already answers the question properly - `os_sdk::runtime::socket_path`
honours the pin if there is one and derives the per-user path otherwise - so the
rule is simply that nobody re-derives it by hand.

WHAT IS ALLOWED. The SDK's own resolver, obviously, and the documentation and
comments that explain the history (including this file's own quotes). What is not
allowed is a literal in code: a `const`, a `Path::new(...)`, an `unwrap_or_else`
fallback. Those are the shapes that shipped.
"""

import re
import sys
from pathlib import Path

OWN_TREE = len(sys.argv) <= 1
REPO = Path(__file__).resolve().parents[2] if OWN_TREE else Path(sys.argv[1]).resolve()

# Where a hardcoded path would actually be dialled.
#
# `dev/` is in here since the first run of this gate, and its absence was not
# hypothetical: `dev/dogfood` fell from the pin straight to /run/arlen, so the
# driver that produces every `file.opened` on the image dialled nothing after the
# bus moved. It was excluded on the reasoning that harnesses pin deliberately -
# but pinning deliberately means setting the variable, which the SDK honours
# first. Nothing needs to write the fallback out.
SCAN = ("apps", "daemons", "dev")
SKIP = ("/target/", "node_modules", "/.git/", "mkosi.builddir")

# The literal, in any of the spellings that reach a connect().
LITERAL = re.compile(r'"(?P<path>/run/arlen/event-bus-[a-z]+\.sock)"')

# The one crate allowed to name it: the SDK's own resolver decides the fallback.
ALLOWED_PREFIXES = ("sdk/os-sdk",)

# Files whose literal is the SYSTEM-CONTEXT branch, reached only after the
# per-user path has been tried. That is the same order the SDK uses, reproduced by
# hand because neither crate depends on os-sdk and one path helper does not earn a
# dependency. The entry is the promise that the XDG check comes first - which is
# exactly what installd got wrong before this check existed, falling from the pin
# straight to /run/arlen and dialling nothing on a per-user system.
XDG_GUARDED = {
    "daemons/installd/installd/src/event_emit.rs",
    "daemons/notification-daemon/src/events/consumer.rs",
}


def offending_lines(path: Path) -> list[tuple[int, str]]:
    """Literal uses, excluding comment lines - a comment explaining the history
    is the opposite of the defect and banning it would delete the explanation."""
    out = []
    for n, line in enumerate(path.read_text(encoding="utf-8", errors="replace").splitlines(), 1):
        stripped = line.lstrip()
        # Everything below `#[cfg(test)]` is test code, and a test that asserts
        # the pinned branch resolves to the pinned path is checking the resolver
        # rather than hardcoding a destination. Banning those would delete the
        # coverage that proves the pin still works.
        if stripped.startswith("#[cfg(test)]"):
            break
        if stripped.startswith("//") or stripped.startswith("#"):
            continue
        if LITERAL.search(line):
            out.append((n, stripped[:100]))
    return out


def main() -> int:
    files = []
    for root in SCAN:
        base = REPO / root
        if not base.is_dir():
            continue
        files.extend(
            f for f in base.rglob("*.rs") if not any(s in str(f) for s in SKIP)
        )
    if not files:
        print(f"NOTHING WAS READ: no Rust source under {REPO}", file=sys.stderr)
        return 2

    problems = []
    for f in sorted(files):
        rel = f.relative_to(REPO)
        if str(rel).startswith(ALLOWED_PREFIXES) or str(rel) in XDG_GUARDED:
            continue
        for n, text in offending_lines(f):
            problems.append(f"{rel}:{n}: {text}")

    if problems:
        print("the event bus's path is written down instead of asked for:")
        for p in problems:
            print(f"  {p}")
        print(
            "\n  Use `os_sdk::runtime::socket_path(\"ARLEN_*_SOCKET\", \"event-bus-*.sock\")`.\n"
            "  It honours a pin when one is set and derives the per-user path when\n"
            "  not, which is the whole difference between following the bus and\n"
            "  guessing where it used to live."
        )
        return 1

    print(f"OK: {len(files)} source file(s), none names a bus socket path itself")
    return 0


if __name__ == "__main__":
    sys.exit(main())
