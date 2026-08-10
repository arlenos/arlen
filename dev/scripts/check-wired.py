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
CANNOT_BE_WIRED = {
    "check-image-contents.sh": (
        "reads a built arlen.raw and answers what is inside it. CI has no image, "
        "and building one to run it would cost more than the question is worth."
    ),
    "probe-webview-sandbox.sh": (
        "starts an app under Xvfb and compares the web process's namespaces "
        "against the app's. Needs a display and a real WebKit process, which the "
        "gate runner has neither of. `check-webview-sandbox.py` covers the source "
        "side and says so."
    ),
    "probe-dbus-gate.sh": (
        "drives a live session bus to watch a refusal happen. Needs a running "
        "daemon on a real bus."
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
    if not SCRIPTS.is_dir():
        print(f"{SCRIPTS} is absent, nothing to check")
        return 0

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

    print(
        f"{wired + excused} check/probe script(s): {wired} run by something, "
        f"{excused} excused with a reason. A mention in prose does not count as a "
        f"run, which is how the last four stayed quiet."
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
