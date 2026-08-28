#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Check that no control deletes a directory it did not mint.

WHY THIS EXISTS, and it is the only check here written after the damage rather than
before it. On 27 August a control in `dev/scripts` passed the REPOSITORY ROOT to a
cleanup helper ending in `rmSync(dir, { recursive: true, force: true })`. It deleted
most of the working tree, `.git` included, and stopped only because one cache
directory was unwritable. Everything committed survived on the remote; one commit
made in the six minutes between the last push and the delete did not.

The pattern that allowed it is the ordinary one in every control here: a helper
takes a path as a PARAMETER and one caller passes a path it did not create. Care is
not the fix - care was already being applied, by someone who had written the helper
ten minutes earlier. The fix is that the delete refuses a path it has no record of
creating, which is what `lib/fixture.mjs` does.

So: a recursive delete in a control must come from that helper. A direct
`rmSync(x, { recursive: true })` is what this refuses.

THE LIST IS EMPTY, and keeping it that way is the point. All 75 controls that
deleted directly were moved onto the helper in two passes on 28 August, each one run
afterwards. The migration found one real thing the old `force: true` had been
swallowing: `test-check-fixtures.mjs` cleaned every fixture twice, which a helper
that keeps a record notices and a flag that means "do not mind if it is already
gone" cannot.

An entry here is a control that deletes on its own, and the list MAY SHRINK AND MAY
NOT GROW. Adding one is saying that a new control may do the thing that cost a
working tree.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = (
    Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
)
SCRIPTS = ROOT / "dev/scripts"
HELPER = "lib/fixture.mjs"

# A recursive delete, in the shapes node offers. `force` is not part of the match:
# it decides whether a missing path is an error, not how much is removed.
DELETE = re.compile(
    r"""(?:rmSync|rm|rmdirSync)\s*\(         # the call
        [^;]*?                                # its arguments, up to the statement end
        recursive\s*:\s*true""",
    re.VERBOSE | re.DOTALL,
)

# A recursive remove reached through a shell, which takes just as much.
#
# Anchored on the CALL and not on the string. The first cut matched any quoted
# `rm -rf`, which meant a comment explaining this rule reported the file it was
# written in - measured on this check's own control. A prose mention is not a
# delete; an argument to `exec`/`spawn` is.
SHELL_DELETE = re.compile(
    r"""(?:exec|spawn)[A-Za-z]*\s*\(     # the call that reaches a shell
        [^)]*?                            # its arguments so far
        ["'`]\s*rm\s+-[a-zA-Z]*[rR]""",
    re.VERBOSE,
)


def without_strings(text: str) -> str:
    """The source with every string literal blanked out.

    A control that TESTS this check has to write a bad delete into a fixture, and a
    scanner that cannot tell code from a quoted description of code reports the
    control as the offender - which is exactly what happened the first time this
    ran. The same technique the Cypher token scanner in the knowledge daemon uses,
    for the same reason.

    Quotes are replaced rather than removed so every offset still lines up, and an
    escaped quote inside a literal does not end it.
    """
    out = []
    quote = None
    escaped = False
    for c in text:
        if quote:
            out.append(" " if c != "\n" else "\n")
            if escaped:
                escaped = False
            elif c == "\\":
                escaped = True
            elif c == quote:
                quote = None
            continue
        if c in "\"'`":
            quote = c
            out.append(" ")
            continue
        out.append(c)
    return "".join(out)


def controls() -> list[Path]:
    """Every control script, which is what may hold a fixture delete."""
    return sorted(p for p in SCRIPTS.glob("*.mjs") if p.name.startswith("test-"))


def offenders(code: str, raw: str) -> list[str]:
    """The unguarded deletes in one file, as the snippet that matched.

    `code` has string literals blanked; `raw` does not. See the call site for why
    the two rules are given different views of the same file.
    """
    found = []
    for m in DELETE.finditer(code):
        found.append(" ".join(m.group(0).split())[:80])
    for m in SHELL_DELETE.finditer(raw):
        found.append(" ".join(m.group(0).split())[:80])
    return found


# Controls that delete on their own. EMPTY, and the header says why it should stay
# that way. MAY SHRINK, MAY NOT GROW.
MIGRATED_LATER: set[str] = set()


def main() -> int:
    files = controls()
    if not files:
        print("no control scripts found; the layout moved and this check did not")
        return 1

    problems: list[str] = []
    still_offending: set[str] = set()
    checked = 0
    present = {n for n in MIGRATED_LATER if (SCRIPTS / n).is_file()}
    # Only in a tree that actually holds these controls. A small fixture holds none
    # of them, and reporting every entry as stale there buries whatever it was
    # testing - the same trap the plugin-grant check fell into.
    stale = sorted(MIGRATED_LATER - present) if present else []

    for path in files:
        raw = path.read_text(encoding="utf-8", errors="replace")
        checked += 1
        # Each rule reads the form its target can appear in. The node call is CODE,
        # so it is matched with strings blanked out; `rm -rf` reaches a shell only
        # ever AS a string, so blanking would make it unfindable.
        found = offenders(without_strings(raw), raw)
        if not found:
            continue
        if path.name in MIGRATED_LATER:
            still_offending.add(path.name)
            continue
        problems.append(
            f"{path.relative_to(ROOT)} deletes recursively on its own "
            f"({found[0]}). A control may only remove what it minted: import "
            f"`mint` and `cleanup` from `{HELPER}`, which refuses a path it has no "
            f"record of creating. This is the check written after one of these "
            f"deleted the repository."
        )

    for name in stale:
        problems.append(
            f"{name} is listed in MIGRATED_LATER and is not there any more. "
            f"Drop the entry: a list of files that do not exist hides how much is left."
        )

    # An entry that no longer offends is an entry that has been migrated, and a
    # list that keeps it reports a backlog that is smaller than it says. This is
    # what makes the count go down rather than the list go quiet.
    if present:
        for name in sorted(present - still_offending):
            problems.append(
                f"{name} no longer deletes on its own and is still listed in "
                f"MIGRATED_LATER. Drop the entry, so the number left to migrate is "
                f"the real one."
            )

    if problems:
        for p in problems:
            print(p)
        return 1

    print(
        f"{checked} control(s): every recursive delete goes through {HELPER}, "
        f"{len(MIGRATED_LATER)} still to migrate"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
