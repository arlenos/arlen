#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""The integration nightly must build every component its suite needs.

A scenario whose binary is absent does not fail. It prints a SKIP line and
returns, and the run exits 0 - which is the right behaviour for a developer
running the suite on a laptop, and exactly the wrong one for the nightly, whose
whole job is to be the backend-health signal.

So a component left out of the recipe's build loop is a set of scenarios that
quietly never run behind a green report. On 14 September that was four consent
scenarios, the config-broker one and the store-backend one: the recipe built five
components, the suite needs eight, and nobody had reason to notice because the
missing ones announced themselves only in a log nobody reads on a pass.

The recipe immediately below this one in the justfile already carries a comment
about the same shape - a test filter that matched nothing, so the go-live gate
"built four crates, ran zero tests and exited 0". Twice is a class.

WHAT IS COMPARED. Every component path the suite names, whether it spawns the
binary (`stack.spawn("daemons/x", "arlen-x", ...)`) or only asks whether it
exists (`binary_built("daemons/x", "arlen-x")`), against the shell `for c in ...`
list in the `integration-nightly` recipe. One direction only: a component in the
recipe that the suite has stopped naming is harmless (a wasted build), and
flagging it would make removing the last scenario for something a two-file change
for no safety.
"""

import re
import sys
from pathlib import Path

OWN_TREE = len(sys.argv) <= 1
REPO = Path(__file__).resolve().parents[2] if OWN_TREE else Path(sys.argv[1]).resolve()

SUITE = "dev/integration/tests/integration_backend_smoke.rs"
JUSTFILE = "dev/justfile"

#: `stack.spawn("daemons/event-bus", "event-bus", &[])` and the logged variant.
SPAWN = re.compile(r'\.spawn(?:_logged)?\(\s*"(?P<component>[^"]+)"\s*,\s*"[^"]+"')

#: `binary_built("daemons/audit-daemon", "arlen-auditd")` - a scenario that only
#: asks still needs the binary present to run at all.
ASKED = re.compile(r'binary_built\(\s*"(?P<component>[^"]+)"\s*,\s*"[^"]+"')

#: The recipe's build loop: `for c in a b c \` possibly over continued lines,
#: ending at `; do`.
BUILD_LOOP = re.compile(r"for c in (?P<list>.*?);\s*do", re.S)


def needed(repo: Path) -> set[str]:
    """Component paths the suite spawns or asks about."""
    text = (repo / SUITE).read_text(encoding="utf-8", errors="replace")
    return {m.group("component") for m in SPAWN.finditer(text)} | {
        m.group("component") for m in ASKED.finditer(text)
    }


def built(repo: Path) -> set[str]:
    """Component paths the `integration-nightly` recipe builds."""
    text = (repo / JUSTFILE).read_text(encoding="utf-8", errors="replace")
    start = text.find("\nintegration-nightly:")
    if start < 0:
        return set()
    # Bound to this recipe: the next line starting at column zero that is not
    # indented and not blank ends it.
    rest = text[start + 1 :]
    end = len(rest)
    for m in re.finditer(r"\n(?=[^\s#])", rest):
        if m.start() > 0:
            end = m.start()
            break
    body = rest[:end]
    loop = BUILD_LOOP.search(body)
    if not loop:
        return set()
    # A component is a path-ish token. Anything with a shell metacharacter in it
    # is loop syntax, not a component - and a plain `w.isalnum()` test drops
    # `store-backend` for its hyphen, which is how this check first reported a
    # component missing that was sitting in the list it was reading.
    words = loop.group("list").replace("\\", " ").split()
    return {w for w in words if re.fullmatch(r"[A-Za-z0-9._/-]+", w)}


def main() -> int:
    want = needed(REPO)
    if not want:
        print(f"NOTHING WAS READ: no component is spawned by {SUITE}", file=sys.stderr)
        return 2
    have = built(REPO)
    if not have:
        print(
            f"NOTHING WAS READ: the `integration-nightly` build loop in {JUSTFILE} did not parse",
            file=sys.stderr,
        )
        return 2

    missing = sorted(want - have)
    if missing:
        print("Components the integration suite needs that the nightly does not build:\n")
        for c in missing:
            print(f"  - {c}")
        print(
            "\nTheir scenarios skip themselves and the run still exits 0, so the nightly"
            "\nreports green having never exercised them. Add each to the `for c in ...`"
            f"\nloop in {JUSTFILE}'s `integration-nightly`."
        )
        return 1

    print(
        f"the integration nightly builds all {len(want)} component(s) its suite names"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
