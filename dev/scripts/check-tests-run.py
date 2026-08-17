#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a component with test files also has the script that runs them.

`check-wired.py` asks this of `dev/scripts/`: a check nothing runs is
indistinguishable from a check that passes. The same hole exists one layer up and
nothing was watching it. The frontend CI job runs `npm test` for a component that declares one, varying
only the flag by runner:

    if npm pkg get scripts.test | grep -q vitest; then npm test -- --passWithNoTests
    else npm test; fi

**Any runner counts.** The first draft of this file required vitest and reported
`ai/pi-plugins`, which deliberately uses `node --test` and whose tests do run -
the `else` branch runs them. A gate asserting a rule CI does not have is worse
than none, since the fix it demands is a change nobody needed.

So a component can hold a directory of tests, sit inside the CI matrix, and
contribute nothing to a green run - and the run looks the same as one where the
tests passed. That is exactly the shape `apps/desktop-shell` was in on 17 August:
in the matrix, `--passWithNoTests`, zero test files, and the app with the most
surfaces in it. The tests came later; the script is what made them count.

This is the cheap half of the rule. It does NOT ask whether a component ought to
have tests - that is a judgement about each one, and several are thin enough that
tests would be ceremony. It asks only that a component which HAS them runs them,
which is not a judgement at all.

Run: dev/scripts/check-tests-run.py [tree]
"""

import json
import sys
from pathlib import Path

#: Where a component's tests live, by convention in this tree.
TEST_SUFFIXES = (".test.ts", ".test.js", ".spec.ts")


def has_tests(component: Path) -> list[Path]:
    """Test files under the component's `src/`, which is where they are written."""
    src = component / "src"
    if not src.is_dir():
        return []
    return [p for p in src.rglob("*") if p.name.endswith(TEST_SUFFIXES)]


def runs_tests(component: Path) -> bool:
    """Whether the component declares a `test` script at all, any runner."""
    manifest = component / "package.json"
    if not manifest.is_file():
        return False
    try:
        scripts = json.loads(manifest.read_text()).get("scripts", {})
    except json.JSONDecodeError:
        return False
    return bool(str(scripts.get("test", "")).strip())


def main() -> int:
    root = Path(sys.argv[1] if len(sys.argv) > 1 else ".")
    components = sorted(
        {p.parent for p in root.glob("*/*/package.json")}
        | {p.parent for p in root.glob("*/*/*/package.json")}
    )
    components = [c for c in components if "node_modules" not in str(c)]

    checked = 0
    silent: list[str] = []
    for c in components:
        tests = has_tests(c)
        if not tests:
            continue
        checked += 1
        if not runs_tests(c):
            rel = c.relative_to(root)
            silent.append(f"{rel}: {len(tests)} test file(s) and no `test` script to run them")

    if checked == 0:
        print("NOTHING WAS READ: no component with test files was found", file=sys.stderr)
        return 2

    if silent:
        print("tests that nothing runs:", file=sys.stderr)
        for s in silent:
            print(f"  {s}", file=sys.stderr)
        print(
            '\nAdd a `test` script to that package.json - `"vitest run"` for a Svelte'
            "\napp, whatever the component already uses otherwise. CI runs it either way.",
            file=sys.stderr,
        )
        return 1

    print(f"check-tests-run: {checked} component(s) with tests all declare a script to run them")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
