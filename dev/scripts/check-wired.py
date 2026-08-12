#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that every check, probe and smoke script is actually run by something.

A check nothing runs is indistinguishable from a check that passes. It is the
same silence the other gates exist to remove, wearing the costume of coverage:
the file is there, it is correct, it has a careful docstring, and it has never
once spoken.

Four had happened before this was written, and the fourth is what prompted it:

  * `check-sensing-vectors.sh` - the only thing comparing the sensing switch
    table across this repo and the compositor, and the only mentions of it
    anywhere were inside another gate's prose. Wired 11 Aug.
  * `probe-dbus-gate.sh` and `probe-webview-sandbox.sh` - two runtime sandbox
    probes, one of them named in a docstring as the thing that does the part the
    static check cannot, and neither reachable by any recipe.
  * the pair before those, in the same shape.

**A mention is not a run**, which is the distinction this makes and a plain grep
does not: `check-webview-sandbox.py` names `probe-webview-sandbox.sh` in its
docstring to say what it cannot do itself, and that reads as a reference while
running nothing. So an invocation has to look like one - a run list naming it, or
a line that executes it.

The exception list is for scripts that CANNOT be wired, not for ones nobody got
round to, and each carries the reason. A runtime probe that needs a display, a
live bus or a built image is a real tool that CI cannot host; saying so is
honest. Saying nothing is how the four above survived.

Run: dev/scripts/check-wired.py [tree]
"""

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

SCRIPTS = ROOT / "dev/scripts"

# Where a run can be declared. Anything naming a script in one of these is wired.
RUN_LISTS = [
    ROOT / ".github/workflows/ci.yml",
    ROOT / "dev/justfile",
    ROOT / ".githooks/pre-commit",
]

# name -> why it cannot be wired. Empty is not the goal here; accuracy is. Each
# entry is a claim that a run list has nowhere to put this.
#
# `check-image-contents.sh` was here until 11 Aug on the ground that CI has no
# image. True, and it was the wrong conclusion: CI is not the only run list, and
# the script's own behaviour was what made it unwireable rather than anything
# about the runners. It errored when the default image was absent, so any list
# that called it would fail on a tree that had never built one. Teaching it to
# tell "you named an image and it is not there" (an error) from "you named none
# and none is built" (nothing to inspect) made it free to call, and `just
# check-executor` now does. The excuse had been sitting here describing a
# limitation that could simply be removed.
CANNOT_BE_WIRED = {
    "probe-webview-sandbox.sh": (
        "starts an app under Xvfb and compares the web process's namespaces "
        "against the app's. Needs a display and a real WebKit process, which the "
        "gate runner has neither of. `check-webview-sandbox.py` covers the source "
        "side and says so."
    ),
}

# Scripts that are libraries or setup helpers rather than checks.
NOT_A_CHECK = re.compile(r"^(dev-|install-|uninstall-|reset-|start-|sync-|run-ci-gates)")

# An invocation, not a mention: a run list entry, or a line that executes it.
def invoked_by(name: str) -> list[str]:
    """Files that RUN `name`, as opposed to talking about it."""
    hits = []
    for path in RUN_LISTS:
        if path.is_file() and name in path.read_text(encoding="utf-8", errors="replace"):
            hits.append(str(path.relative_to(ROOT)))

    listed = subprocess.run(
        ["git", "ls-files", "*.sh", "*.py", "*.mjs", "*.yml", "justfile"],
        cwd=ROOT, capture_output=True, text=True, check=False,
    ).stdout.split()
    call = re.compile(rf"(?:^|[|&;(]|\b(?:bash|sh|python3?|node|exec)\s+)\S*{re.escape(name)}\b")
    for rel in listed:
        if Path(rel).name == name:
            continue
        # A gate's own positive control contains example invocations as FIXTURE
        # TEXT - `test-check-wired.mjs` writes a justfile that calls a probe, to
        # watch this gate react. Counting those as runs would let a check be
        # "wired" by nothing but a test that plants a string, which is precisely
        # the mention-is-not-a-run distinction this file exists to draw, one turn
        # further in.
        if Path(rel).name.startswith("test-check-"):
            continue
        p = ROOT / rel
        if not p.is_file():
            continue
        for line in p.read_text(encoding="utf-8", errors="replace").splitlines():
            stripped = line.strip()
            if stripped.startswith("#") or stripped.startswith("//"):
                continue
            if call.search(line):
                hits.append(rel)
                break
    return sorted(set(hits))


def main() -> int:
    # This check exists because a gate nobody runs reads exactly like a gate that
    # passes, so it would be a poor joke for it to have the same hole. The scripts
    # directory is committed source: its absence is a wrong root argument, not a
    # tree that has none.
    if not SCRIPTS.is_dir():
        print(f"NOTHING WAS READ: {SCRIPTS} is absent", file=sys.stderr)
        return 2

    problems, wired, excused = [], 0, 0
    for script in sorted(SCRIPTS.iterdir()):
        name = script.name
        if not script.is_file() or NOT_A_CHECK.match(name):
            continue
        if not (name.startswith(("check-", "test-", "probe-", "smoke-"))):
            continue
        if invoked_by(name):
            wired += 1
            continue
        if name in CANNOT_BE_WIRED:
            excused += 1
            continue
        problems.append(
            f"{name}: nothing runs it. A check no run list names cannot fail, "
            f"which makes it indistinguishable from one that passes. Put it in "
            f"CI, the justfile or the pre-commit sweep, or add it to "
            f"CANNOT_BE_WIRED with the reason it cannot go in one."
        )

    if wired + excused + len(problems) == 0:
        print(
            f"NOTHING WAS READ: {SCRIPTS} holds no check, probe or smoke script",
            file=sys.stderr,
        )
        return 2

    # An excuse outliving its reason is the same defect one level up, so both
    # ways of going stale are named: the script is gone, or it turns out to be
    # wired after all and the entry is now claiming something untrue about it.
    for n in sorted(CANNOT_BE_WIRED):
        if not (SCRIPTS / n).is_file():
            problems.append(f"{n} is excused from being wired but no longer exists; delete the entry")
        elif invoked_by(n):
            problems.append(
                f"{n} is excused as unwireable but {', '.join(invoked_by(n))} runs it; "
                f"delete the entry"
            )

    # How many checks can be shown to fail, counted rather than remembered.
    #
    # This is informational and does NOT fail the run: writing the missing
    # controls as a batch is an evening on scaffolding, so they land when their
    # subject is next touched. But a gap nobody measures is a gap that grows, and
    # a number in a README is a claim that stops being true the day after it is
    # written - so it is printed here, from the tree, every time.
    checks = [
        f.name
        for f in sorted(SCRIPTS.iterdir())
        if f.is_file() and f.name.startswith("check-") and not NOT_A_CHECK.match(f.name)
    ]
    # Which check each control DRIVES, read from the file, rather than inferred
    # from its name. `test-check-fixtures.mjs` drives `check-invoke-shape.py` and
    # `test-check-units.mjs` drives `check-packaged-units.sh`; a name-matching
    # version of this listed both of those checks as unproven, which is the same
    # mistake as everything else on this page - measuring the label instead of
    # the thing.
    driven = set()
    for t in SCRIPTS.glob("test-*.mjs"):
        for m in re.finditer(r"check-[a-z0-9-]+\.(?:py|sh|mjs)", t.read_text(encoding="utf-8", errors="replace")):
            driven.add(m.group(0))
    proven = [c for c in checks if c in driven]
    print(
        f"{len(proven)} of {len(checks)} check(s) have a positive control - a "
        f"test that plants a defect and watches them fail. The rest are not "
        f"disproven, only unproven; they gain one when their subject is next "
        f"touched."
    )
    if len(proven) < len(checks):
        missing = [c for c in checks if c not in proven]
        print(f"  without one: {', '.join(missing)}")
    print()

    print(
        f"{wired + excused} check/probe script(s): {wired} run by something, "
        f"{excused} excused with a reason. A mention in prose does not count as a "
        f"run, which is how the last four stayed quiet."
    )

    # Which of them a PUSH actually runs. This gate accepts a hit in any run list,
    # so a check named only in the justfile counts as wired while never running in
    # CI - the same "it is there, so it must be working" shape one level up, and it
    # hid a real one: `test-check-invoke-scope.mjs`, added to `just checks` on
    # 12 Aug and to CI only after this line was written and printed the answer.
    #
    # Informational, not a failure. Some genuinely cannot run on a push: the image
    # and smoke scripts need a built image or live daemons. Naming them every run
    # is cheaper than a second exemption list, and it keeps the number honest
    # rather than remembered.
    ci = ROOT / ".github/workflows/ci.yml"
    if ci.is_file():
        ci_text = ci.read_text(encoding="utf-8", errors="replace")
        local_only = sorted(
            f.name
            for f in SCRIPTS.iterdir()
            if f.is_file()
            and f.name.startswith(("check-", "test-", "probe-", "smoke-"))
            and not NOT_A_CHECK.match(f.name)
            and f.name not in CANNOT_BE_WIRED
            and invoked_by(f.name)
            and f.name not in ci_text
        )
        if local_only:
            print(
                f"\n{len(local_only)} run locally but NOT on a push: "
                f"{', '.join(local_only)}"
            )
        # And naming them is not enough: an honest list still has to be someone's
        # job. `boot-verify-payload.tsv` says it is the boot-verify run's, and the
        # two sets are held equal here so the declaration cannot rot into a note
        # about a set that has moved on. A check that becomes push-runnable must
        # leave the payload; one that stops being push-runnable must join it.
        payload_file = SCRIPTS / "boot-verify-payload.tsv"
        if not payload_file.is_file():
            problems.append(
                "boot-verify-payload.tsv is missing, so the checks that cannot run "
                "on a push have no declared home and nothing would notice"
            )
        else:
            declared = {
                line.split("\t")[0].strip()
                for line in payload_file.read_text(encoding="utf-8").splitlines()
                if line.strip() and not line.startswith("#")
            }
            for name in sorted(declared - set(local_only)):
                problems.append(
                    f"{name} is in the boot-verify payload but is not one of the "
                    f"checks that only run locally - it runs on a push, or it no "
                    f"longer exists. Drop it from the payload."
                )
            for name in sorted(set(local_only) - declared):
                problems.append(
                    f"{name} runs locally but on no push, and the boot-verify "
                    f"payload does not name it - so nothing would ever run it "
                    f"against an image. Add it, with what it proves."
                )

    if problems:
        print("\nwritten but never run:\n")
        for p in problems:
            print(f"  - {p}")
        return 1
    for name in sorted(CANNOT_BE_WIRED):
        print(f"CANNOT BE WIRED: {name}: {CANNOT_BE_WIRED[name]}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
