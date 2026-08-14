#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""One environment variable, declared in three crates, agreeing by comment.

A handoff across a process boundary is a contract with no compiler behind it. The
greeter sets `ARLEN_A11Y_SCREEN_READER`, the session carries it into the systemd
import list, and the shell reads it and writes the result to the user's config.
Three crates that cannot dep each other for good reasons - the shell has no
business linking the login screen, and the session daemon has no business linking
either - so each declares the string itself, with a comment saying it matches the
others.

That comment is the whole guarantee. Rename it in one place and nothing fails to
compile, no test goes red, and the symptom is that somebody's screen reader stops
following them from the login screen into their session. Silent, and the kind of
silence that reads as "the feature was never wired up".

So each handoff records WHO has to spell it, and the check reads the tree to
confirm they all still do. Rename one side and that crate drops out of the group,
which goes red here at the moment it happens rather than at the moment somebody
notices their screen reader stopped following them.

WHY A RECORDED LIST AND NOT PURE DERIVATION: the obvious version groups every
declaration by its VALUE and reports the groups - and can never fail, because
everything in a group agrees by construction. That is a test that cannot fail
wearing a gate's clothes, and it was the first thing built here before the flaw
was noticed. Knowing who BELONGS in the group is what makes a rename visible.

WHY BY VALUE AND NOT BY CONST NAME: the three consts are called
`A11Y_SCREEN_READER_ENV`, `A11Y_SCREEN_READER` and `HANDOFF_ENV` - each named for
what it means where it lives, which is right. The string is what has to match, so
the string is what this groups on.
"""

import re
import sys
from collections import defaultdict
from pathlib import Path

OWN_TREE = len(sys.argv) <= 1
REPO = Path(__file__).resolve().parents[2] if OWN_TREE else Path(sys.argv[1]).resolve()

ROOTS = ("apps", "daemons", "sdk", "ai", "contracts", "forage")
SKIP = ("/target/", "node_modules", "/.git/", "mkosi.builddir")

# A const holding an env-var name. `ARLEN_*` only: the tree's own namespace, so a
# third-party name a crate happens to read (`HOME`, `XDG_*`) is not this check's
# business - those are somebody else's contract and we do not get to police them.
ENV_CONST = re.compile(r'const\s+(\w+)\s*:\s*&str\s*=\s*"(ARLEN_[A-Z0-9_]+)"')

# The handoffs that cross a process boundary, and who has to spell them.
#
# RECORDED rather than derived, because the derivation cannot fail: grouping
# declarations by their VALUE means everything in a group agrees by construction,
# so a check built that way lists and never goes red - which is the shape of a
# test that cannot fail, wearing a gate's clothes. What catches a rename is
# knowing who is SUPPOSED to be in the group: rename one side and that crate
# leaves it, which this sees.
#
# An entry says why the name crosses, so the next reader can tell a real handoff
# from a coincidence.
SHARED: dict[str, tuple[set[str], str]] = {
    "ARLEN_A11Y_SCREEN_READER": (
        {"apps/greeter", "daemons/session/src", "apps/desktop-shell/src-tauri"},
        "the login screen's screen-reader choice: the greeter sets it, the session "
        "carries it into the systemd import list, the shell reads it once and writes "
        "it to that user's config broker. None of the three may dep the others - the "
        "shell has no business linking the login screen - so the string is the "
        "contract.",
    ),
}


def crate_of(repo: Path, path: Path) -> str:
    """The crate directory a file belongs to, as its identity for grouping."""
    rel = path.relative_to(repo).parts
    # apps/<app>/... and daemons/<d>/..., but apps/greeter/core and
    # apps/greeter/src-tauri are separate crates and have to count separately.
    for depth in (3, 2):
        if len(rel) > depth and rel[depth - 1] in ("src", "src-tauri"):
            return "/".join(rel[:depth])
    return "/".join(rel[:2])


def main() -> int:
    files = [
        f
        for root in ROOTS
        if (REPO / root).is_dir()
        for f in (REPO / root).rglob("*.rs")
        if not any(s in str(f) for s in SKIP)
    ]
    if not files:
        print(f"NOTHING WAS READ: no Rust sources under {REPO}", file=sys.stderr)
        return 2

    # value -> {crate -> {const names}}
    declared: dict[str, dict[str, set[str]]] = defaultdict(lambda: defaultdict(set))
    for f in sorted(files):
        text = f.read_text(encoding="utf-8", errors="replace")
        for name, value in ENV_CONST.findall(text):
            declared[value][crate_of(REPO, f)].add(f"{name} ({f.relative_to(REPO)})")

    problems: list[str] = []

    for value, (expected, why) in SHARED.items():
        actual = set(declared.get(value, {}))
        missing = expected - actual
        if missing:
            sites = "\n".join(
                f"      {c}: {', '.join(sorted(declared[value][c]))}" for c in sorted(actual)
            )
            problems.append(
                f"{value} is no longer spelled by {', '.join(sorted(missing))}.\n"
                f"    {why}\n"
                f"    Still declared by:\n{sites or '      nothing'}\n"
                f"    A rename on one side of a handoff compiles and passes. If this "
                f"was deliberate, rename EVERY side and update SHARED; if not, the "
                f"handoff is broken right now and nothing else will say so."
            )
        extra = actual - expected
        if extra:
            problems.append(
                f"{value} is now also declared by {', '.join(sorted(extra))}.\n"
                f"    A fourth speaker of a three-way contract is either a new "
                f"participant (add it to SHARED, with why) or a name collision that "
                f"will surprise somebody."
            )

    # A name shared across crates that nobody recorded. Not a failure - it may be
    # a coincidence - but it is exactly what SHARED is for, so say it.
    unrecorded = sorted(v for v, c in declared.items() if len(c) > 1 and v not in SHARED)
    if unrecorded:
        print("shared across crates and not recorded in SHARED:")
        for value in unrecorded:
            print(f"  {value}: {', '.join(sorted(declared[value]))}")
        print()

    if problems:
        print("a cross-process handoff whose name stopped agreeing:")
        for p in problems:
            print(f"  {p}")
        return 1

    print(f"OK: {len(SHARED)} recorded handoff name(s), each spelled by every crate that must")
    return 0


if __name__ == "__main__":
    sys.exit(main())
