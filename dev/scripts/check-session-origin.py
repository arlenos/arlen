#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A launcher that starts producers must supply the session id they now read.

`origin` on an event is either a session reference or `system:<producer>`, and
producers READ the session id rather than minting one - one login is one session,
so one thing mints it. The consequence nobody has to think about until it bites:
a producer that finds `ARLEN_SESSION_ID` unset sends an EMPTY origin, and the bus
refuses an empty origin. Every event from that launcher is dropped.

That is not hypothetical. On 10 August the producers were taught to read the id
and the launchers were not updated to supply it. Ten integration tests went red
the same afternoon - the harness minted nothing - and all three `just dev` stacks
had the identical hole, which no CI watches, so it would have been found by hand
at whatever moment someone next needed the dev stack to work.

The rule, deliberately narrow so it does not turn into a list of exceptions:

    a process-compose file that starts any process must set ARLEN_SESSION_ID in
    its top-level `environment:`, and `arlen-session` must still mint one.

Narrow because these stacks exist to run the system, so every one of them starts
producers; there is no useful sub-case where a stack of Arlen processes wants its
events refused. The second half pins the minter itself: if that line ever goes,
the id is unset everywhere at once and this check should be what says so.
"""

import re
import sys
from pathlib import Path

# An explicit root so the gate can be pointed at a fixture tree; the sibling gates
# take one for the same reason, and a check nobody can drive is a check nobody has
# seen fail.
REPO = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
VAR = "ARLEN_SESSION_ID"


def compose_files():
    """Every process-compose file in the tree, found rather than listed."""
    return sorted(REPO.glob("dev/process-compose*.yaml"))


def top_level_environment(text):
    """The top-level `environment:` block's entries.

    Hand-parsed on purpose: the gate must not need PyYAML, which is not a
    dependency of anything else in `dev/scripts`. A top-level block starts at
    column zero and runs until the next column-zero key.
    """
    out, inside = [], False
    for line in text.splitlines():
        if re.match(r"^environment:\s*$", line):
            inside = True
            continue
        if inside:
            if line and not line[0].isspace():
                break
            entry = line.strip()
            if entry.startswith("- "):
                out.append(entry[2:].strip().strip('"').strip("'"))
    return out


def starts_processes(text):
    """True if the file declares at least one process under `processes:`."""
    inside = False
    for line in text.splitlines():
        if re.match(r"^processes:\s*$", line):
            inside = True
            continue
        if inside:
            if line and not line[0].isspace():
                break
            if re.match(r"^\s{2}[A-Za-z0-9_.-]+:\s*$", line):
                return True
    return False


def main():
    problems = []
    checked = 0

    for path in compose_files():
        text = path.read_text(encoding="utf-8")
        if not starts_processes(text):
            continue
        checked += 1
        if not any(e.startswith(f"{VAR}=") for e in top_level_environment(text)):
            problems.append(
                f"{path.relative_to(REPO)}: starts processes but never sets {VAR}, "
                f"so every producer it launches sends an empty origin and the bus "
                f"refuses the lot"
            )

    session = REPO / "dev/mkosi/mkosi.extra/usr/bin/arlen-session"
    if session.exists():
        checked += 1
        text = session.read_text(encoding="utf-8")
        if not re.search(rf"^\s*{VAR}=", text, re.M):
            problems.append(
                f"{session.relative_to(REPO)}: no longer mints {VAR}. It is the one "
                f"thing that does, so nothing in a booted session would have one"
            )

    if problems:
        print("session origin: a launcher does not supply the id its producers read")
        for p in problems:
            print(f"  {p}")
        print("  Set it in the launcher. Use the SAME value across stacks that run")
        print("  together: one login is one session, so a fresh mint per stack")
        print("  reintroduces the two-ids-for-one-login this rule exists to prevent.")
        return 1

    print(f"OK: {checked} launcher(s) supply the session id their producers read")
    return 0


if __name__ == "__main__":
    sys.exit(main())
