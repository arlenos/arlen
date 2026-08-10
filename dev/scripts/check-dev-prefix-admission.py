# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that no admission gate admits the whole `dev.` prefix.

A component built from a cargo target directory resolves to `dev.<binary>`, and
every locally-built binary in the tree gets one. So `app_id.starts_with("dev.")`
in a debug build does not mean "this is the development build of the component I
admit" - it means "this is any locally-built binary at all", including a test, a
helper, an experiment, and every other daemon.

Three gates had already been moved off that shape one at a time, each with the
same reasoning written out: the audit ingest, the undo signer, the transfer
daemon. Six more still carried it, and they guarded answering consent prompts,
driving the module runtime, minting a capsule, writing a consent grant, reading
another app's meeting notes, and reaching any first-party MCP server. They are all
exact lists now, which is what this keeps.

The shape is not always harmless-in-debug either. The consent broker's grant
management is split from prompt answering precisely so that Settings can revoke
without being able to answer; the prefix handed a debug build both.

What this checks: no `starts_with("dev.")` outside a test. An exact match, a
`matches!` arm or a named `*_DEV` list is the way to admit a development build.

What it does NOT check:

  * that an exact list contains the RIGHT ids. It compares nothing to nothing;
    that is what each gate's own test is for.
  * `starts_with("dev.arlen-")`, which is narrower and would be an odd but not
    dangerous thing to write.
  * whether a gate should exist at all.

Shown to fail before being trusted: restoring the prefix in any of the six makes
it name that file and line.
"""

import re
import sys
from pathlib import Path

# A tree to check may be passed in, which is what lets this gate's own test drive
# it against fixtures; the sibling gates take the same argument.
ROOT = (
    Path(sys.argv[1]).resolve()
    if len(sys.argv) > 1
    else Path(__file__).resolve().parents[2]
)
SKIP = {"target", "node_modules", "mkosi.builddir", ".git", ".svelte-kit", "build", "dist"}
PATTERN = re.compile(r'starts_with\(\s*"dev\."\s*\)')


def test_scope_lines(lines: list[str]) -> set[int]:
    """Line indices that are genuinely INSIDE a `#[cfg(test)]` item's body.

    A test asserting something about the prefix is talking ABOUT the shape rather
    than admitting by it, so it is excused - but only if it really is a test.

    The predecessor scanned backwards for the nearest `#[cfg(test)]` within 400
    lines and excused anything it found one above. Rust convention puts the test
    module at the END of a file, so that excused every line after it: an
    admission added at the bottom of a file was invisible to this check. Measured
    on 10 August by injecting `starts_with("dev.")` into `event-bus/src/socket.rs`
    twice - above the test module it was caught, appended after it the gate passed.

    So track braces instead. A scope opens at the first `{` after the marker and
    closes when depth returns, which is what "inside" actually means.
    """
    inside: set[int] = set()
    depth = 0
    opened_at: list[int] = []
    pending = False
    for i, line in enumerate(lines):
        if opened_at:
            inside.add(i)
        if "#[cfg(test)]" in line or line.strip().startswith("mod tests"):
            pending = True
        for ch in line:
            if ch == "{":
                depth += 1
                if pending:
                    opened_at.append(depth)
                    pending = False
            elif ch == "}":
                if opened_at and depth == opened_at[-1]:
                    opened_at.pop()
                depth -= 1
    return inside


def main() -> int:
    findings: list[str] = []
    scanned = 0
    for path in sorted(ROOT.rglob("*.rs")):
        if SKIP & set(path.parts):
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        if "dev." not in text:
            continue
        scanned += 1
        lines = text.splitlines()
        in_test = test_scope_lines(lines)
        for n, line in enumerate(lines):
            if not PATTERN.search(line):
                continue
            if n in in_test or "assert" in line:
                continue
            findings.append(
                f"{path.relative_to(ROOT)}:{n + 1}: admits every id starting with "
                f"`dev.`, which in a debug build is every locally-built binary in "
                f"the tree, not the one component this gate is for. Name the exact "
                f"id, or a `*_DEV` list of them."
            )

    print(
        f"{scanned} file(s) mentioning a `dev.` id checked for admitting the whole "
        f"prefix. Whether an exact list holds the right ids is each gate's own test."
    )
    if findings:
        print("\nadmitting the whole development namespace:\n")
        for f in findings:
            print(f"  - {f}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
