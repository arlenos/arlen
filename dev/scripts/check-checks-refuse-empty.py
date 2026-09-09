#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a check refuses a tree it found nothing in.

The trap this closes has a name in this repo already: "0 app(s) checked" also
exits 0. A gate whose scan is pointed at the wrong root, or whose tree moved
under it, reports CLEAN - and a clean report from a gate is the most expensive
false statement the repo can make, because it is the one nobody re-reads. Three
gates were caught with it in September, which is why most of the checks here end
with an explicit "so the scan is pointed wrong" refusal.

MEASURED RATHER THAN ASSUMED, on 10 September: every `check-*.py` was run against
an empty tree with the usual top-level directories and nothing in them. 125
refused. 27 said everything was fine.

WHAT IT DOES. Runs each check against an empty temp tree and requires a non-zero
exit. That is the whole rule: a check that read nothing has nothing to vouch for.

WHAT IT CANNOT SEE, and why the carried list is a queue rather than a hole: a
check may still be vacuous on a tree that is NOT empty - one whose file glob
stopped matching after a rename, say - and no meta-check can tell that from a
tree with nothing to find. This closes the crude half. The other half is why each
check prints what it read.

Run: dev/scripts/check-checks-refuse-empty.py [root]
"""

import subprocess
import sys
import tempfile
from pathlib import Path

#: Checks that pass on an empty tree, measured 10 September. Every one is a
#: defect of the same shape - a scan that found nothing and said nothing about
#: it - and the list comes down as they are fixed. A NEW name here fails.
CARRIED: set[str] = {
    "check-command-shapes-agree.py",
    "check-controls-do-not-write-the-tree.py",
    "check-controls-exist.py",
    "check-dbus-call-arity.py",
    "check-fabricated-verdict.py",
    "check-fixture-answers-whole.py",
    "check-headless-render.py",
    "check-invoke-exists.py",
    "check-message-placeholders.py",
    "check-socket-tiers.py",
}

#: This check itself, which is exempt by construction: pointed at an empty tree
#: it finds no checks to run, and the refusal it would need is the one it is
#: asking of everybody else - handled below rather than by an entry.
SELF = "check-checks-refuse-empty.py"

#: Long enough for the slowest check to read a tree with nothing in it. Generous
#: because the pre-commit hook runs the whole gate set at once: at 30s this was
#: measuring how busy the machine was, and a contended check that crossed it got
#: reported as a verdict it never gave.
TIMEOUT_S = 120


def main() -> int:
    root = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
    scripts = sorted((root / "dev/scripts").glob("check-*.py"))
    if not scripts:
        print("check-checks-refuse-empty: no checks found, so the scan is pointed wrong")
        return 1

    with tempfile.TemporaryDirectory(prefix="arlen-empty-") as tmp:
        empty = Path(tmp)
        # The shape of a checkout with nothing in it. Directories rather than a
        # bare temp dir, so a check that refuses on "no apps directory" is
        # answering about emptiness and not about the layout.
        for d in ("apps", "sdk", "daemons", "dev/scripts", "contracts", "ai"):
            (empty / d).mkdir(parents=True, exist_ok=True)

        vacuous: list[str] = []
        fixed: list[str] = []
        hung: list[str] = []
        for script in scripts:
            if script.name == SELF:
                continue
            try:
                done = subprocess.run(
                    [sys.executable, str(script), str(empty)],
                    capture_output=True,
                    timeout=TIMEOUT_S,
                )
            except subprocess.TimeoutExpired:
                # Not a verdict. A check that did not finish never answered the
                # question this gate asks, so it is reported as what it is
                # rather than folded in with the ones that answered "clean".
                hung.append(script.name)
                continue
            if done.returncode == 0:
                if script.name not in CARRIED:
                    vacuous.append(script.name)
            elif script.name in CARRIED:
                fixed.append(script.name)

    if hung:
        print("Checks that did not answer within the time budget:\n")
        for name in sorted(hung):
            print(f"  - {name}")
        print(
            "\n  This says nothing about whether they refuse an empty tree - they"
            "\n  never got that far. Either the check is genuinely stuck on a tree"
            "\n  with nothing in it, or it is slow enough that the budget measured"
            "\n  the machine. Run it alone against an empty tree to tell which."
        )
        return 1

    if vacuous:
        print("Checks that call an empty tree clean:\n")
        for name in vacuous:
            print(f"  - {name}")
        print(
            "\n  A scan that read nothing has nothing to vouch for. Refuse when the"
            "\n  collection is empty - 'so the scan is pointed wrong' is the sentence"
            "\n  the rest of them use."
        )
        return 1

    if fixed:
        print("Checks that now refuse an empty tree and are still carried:\n")
        for name in sorted(fixed):
            print(f"  - {name}")
        print("\n  Take them out of CARRIED so a new one cannot hide behind the count.")
        return 1

    print(
        f"check-checks-refuse-empty: {len(scripts) - 1} check(s) run against an empty tree; "
        f"{len(CARRIED)} carried, the rest refuse it"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
