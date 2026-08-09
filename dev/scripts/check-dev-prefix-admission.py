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

ROOT = Path(__file__).resolve().parents[2]
SKIP = {"target", "node_modules", "mkosi.builddir", ".git", ".svelte-kit", "build", "dist"}
PATTERN = re.compile(r'starts_with\(\s*"dev\."\s*\)')


def in_test_module(lines: list[str], index: int) -> bool:
    """Whether this line sits under a `#[cfg(test)]` or in a `#[test]` function.

    Crude but adequate: a test that asserts something about the prefix is talking
    ABOUT the shape rather than admitting by it, and the one in the tree does
    exactly that (it checks the release surface list carries no debug id).
    """
    for i in range(index, max(index - 400, -1), -1):
        if lines[i].startswith("mod tests") or "#[cfg(test)]" in lines[i]:
            return True
        if lines[i].startswith("}") and i < index - 1:
            continue
    return False


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
        for n, line in enumerate(lines):
            if not PATTERN.search(line):
                continue
            if in_test_module(lines, n) or "assert" in line:
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
