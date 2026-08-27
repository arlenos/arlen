#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Every check in `dev/scripts` is one CI actually runs.

A check nobody runs cannot be told apart from a check that passes. That is not
hypothetical here: `check-image-contents.sh` sat correct and unread until 11
August, when it turned out to be the one file in this directory nothing invoked,
and its own header now records it.

`run-ci-gates.sh` discovers what to run by grepping the workflow, so a check that
is never named there is invisible to the local runner AND to CI at once. Writing
one and forgetting the registration line leaves a file that looks like coverage.

WHAT IS COMPARED. Every `dev/scripts/check-*.{py,mjs,sh}` against the text of
`.github/workflows/ci.yml`, by filename. One direction only.

THE REVERSE - a control whose check does not exist - is NOT checked, and the two
attempts are worth recording so nobody spends the afternoon again. Keying it on
the control's NAME calls three real controls broken, because a control may be
named for the class rather than the file (`test-check-fixtures.mjs` drives
`check-fixture-on-failure.py`). Keying it on the check paths the control MENTIONS
calls `test-check-wired.mjs` broken, because that control fabricates check files
in a fixture tree and naming them is its whole job. A rule that reports three
healthy files on a clean tree is worse than no rule.

THE TWO THAT ARE NOT IN CI ARE BELOW, each with the reason from its own header.
Both need something a runner does not have - installed desktop software, or a
built image - and both are wired into the justfile instead. The list may shrink
and may not grow: a third entry is a check somebody wrote and nobody runs.
"""

from __future__ import annotations

import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else REPO_ROOT

NOT_IN_CI: dict[str, str] = {
    "check-profile-case.sh": (
        "needs a machine with apps installed to read `.desktop` basenames from, "
        "and CI has none; `just` runs it where real software lives"
    ),
    "check-image-contents.sh": (
        "reads a built `arlen.raw`, which no per-commit CI job produces; it is "
        "wired into `just check-executor` and answers harmlessly when no image "
        "has been built"
    ),
}


def main() -> int:
    scripts = ROOT / "dev/scripts"
    workflow = ROOT / ".github/workflows/ci.yml"
    if not scripts.is_dir() or not workflow.is_file():
        print(
            f"NOTHING WAS READ: need {scripts} and {workflow}",
            file=sys.stderr,
        )
        return 2

    yml = workflow.read_text(encoding="utf-8", errors="replace")
    names = sorted(
        p.name
        for p in scripts.iterdir()
        if p.is_file() and p.name.startswith("check-") and p.suffix in (".py", ".mjs", ".sh")
    )
    if not names:
        print("NOTHING WAS READ: no check-* script found", file=sys.stderr)
        return 2

    findings: list[str] = []
    for name in names:
        registered = name in yml
        if registered and name in NOT_IN_CI:
            findings.append(
                f"{name} is recorded as not-in-CI but the workflow names it now; "
                f"the exception was left behind"
            )
        elif not registered and name not in NOT_IN_CI:
            findings.append(
                f"{name} is not named in ci.yml, so neither CI nor "
                f"`run-ci-gates.sh` runs it. A check nobody runs cannot be told "
                f"apart from one that passes. Register it, or record why it "
                f"cannot run there."
            )

    stale = sorted(k for k in NOT_IN_CI if not (scripts / k).is_file()) if ROOT == REPO_ROOT else []

    print(f"{len(names)} check(s) in dev/scripts, {len(NOT_IN_CI)} recorded as not runnable in CI")
    for s in stale:
        findings.append(f"{s} is recorded as not-in-CI but no longer exists; delete the entry")
    if findings:
        print("\na check nobody runs:\n")
        for f in findings:
            print(f"  - {f}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
