#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A test file has to state what the result should be.

An agent reliably writes the scaffolding of a test - the file, the imports, the
name, the call - and unreliably writes the ORACLE, the line saying what the
answer ought to be. Measured across agents the rate of a strong oracle on newly
created test files runs from 18% to 67%.

The failure is quiet and it is the worst shape there is. A test that calls the
code and asserts nothing passes forever; a test that asserts what the code
CURRENTLY DOES turns an existing bug into a green test, and every later run
defends it. Neither shows up as a red anywhere.

**This flags the absent oracle and nothing else.** Whether an assertion is a good
one is the part no check can do - `assert!(true)` passes here - and pretending
otherwise would make it a gate people argue with instead of one they trust.

What it reads, and each kind's assertion vocabulary is its own:

  * `*.rs` carrying `#[test]` or `#[tokio::test]`: `assert!`, `assert_eq!`,
    `assert_ne!`, `debug_assert!`, `panic!(`, `#[should_panic]`.
  * `*.test.ts` / `*.test.js` / `*.spec.ts`: `expect(` or `assert`.
  * `dev/scripts/test-*.mjs`, the gate controls: their oracle is not the word
    `assert` at all. Each one counts failures through its own `check()` or
    `ok()`/`bad()` helper and ends `process.exit(failures ? 1 : 0)`, and the gate
    runner reads that exit code. So what makes a control a control is a path that
    can exit NON-ZERO; one that cannot always passes and proves nothing.

    The first cut of this file looked for `ok(` and `bad(` and reported sixty of
    the hundred and fifty-six, because the helper is named differently in about
    half of them. That is this directory's own recurring lesson - a check matches
    the shape its author last happened to write - caught here by measuring the
    population before believing the pattern.

**On the corpus bound.** The job that asked for this said new-or-changed files
rather than the whole tree, so it could not flood on legacy. Measured before
building it: 863 Rust test files, 88 vitest, 156 controls, and **one** without an
assertion pattern. There is nothing to flood with, so it reads everything, which
is both simpler and stronger than a diff against a base ref that a standalone run
does not have. If that ever stops being true the bound goes back in.

`KNOWN` is the escape the same job named: a file whose oracle is real but not a
literal assertion says so there, with the reason, once.
"""

import os
import pathlib
import re
import sys

ROOT = (
    pathlib.Path(sys.argv[1]).resolve()
    if len(sys.argv) > 1
    else pathlib.Path(__file__).resolve().parents[2]
)

RUST_TEST = re.compile(r"#\[(?:tokio::)?test\]")
RUST_ASSERT = re.compile(r"\bassert(?:_eq|_ne)?!|\bdebug_assert(?:_eq|_ne)?!|\bpanic!\s*\(|should_panic")
TS_ASSERT = re.compile(r"\bexpect\s*\(|\bassert\b")
# A control's verdict is its exit code, so its oracle is a path that can exit
# non-zero. `process.exit(0)` alone is a script that always says yes.
CONTROL_ASSERT = re.compile(
    r"process\.exit\(\s*(?!0\s*\))|process\.exitCode\s*=\s*(?!0\b)"
)

SKIP_DIRS = {
    "target",
    "node_modules",
    ".git",
    ".svelte-kit",
    "build",
    "dist",
    ".vite",
    "mkosi.builddir",
    # The vendored Python standard library that mkosi carries. It has its own
    # test suite, it is not ours, and reading it added forty seconds.
    "mkosi.tools",
}

# path -> why its oracle is real without a literal assertion. One line each, and
# the reason has to say what the test would catch, not that it is fine.
KNOWN = {
    "daemons/ai-engine-daemon/src/graph_adapter.rs": (
        "the oracle is the absence of a panic: `OsSdkGraphQuerier::new` is lazy, "
        "so constructing one against a socket that does not exist must return "
        "rather than dial or block, and the test body completing IS that claim"
    ),
}


def kind_of(path: pathlib.Path, text: str) -> str | None:
    """Which test vocabulary a file speaks, or None if it is not a test file."""
    name = path.name
    if name.endswith(".rs"):
        return "rust" if RUST_TEST.search(text) else None
    if name.endswith((".test.ts", ".test.js", ".spec.ts")):
        return "vitest"
    if name.startswith("test-") and name.endswith(".mjs"):
        return "control"
    return None


PATTERN = {"rust": RUST_ASSERT, "vitest": TS_ASSERT, "control": CONTROL_ASSERT}


def main() -> int:
    counted = {"rust": 0, "vitest": 0, "control": 0}
    findings: list[str] = []
    excused: list[str] = []

    for base, dirs, files in os.walk(ROOT):
        dirs[:] = [d for d in dirs if d not in SKIP_DIRS]
        for name in sorted(files):
            if not name.endswith((".rs", ".ts", ".js", ".mjs")):
                continue
            path = pathlib.Path(base) / name
            try:
                text = path.read_text(encoding="utf-8", errors="replace")
            except OSError:
                continue
            kind = kind_of(path, text)
            if kind is None:
                continue
            counted[kind] += 1
            if PATTERN[kind].search(text):
                continue
            rel = str(path.relative_to(ROOT))
            if rel in KNOWN:
                excused.append(f"{rel}: {KNOWN[rel]}")
                continue
            why = (
                "no path that exits non-zero, so the gate runner reads success "
                "whatever it found"
                if kind == "control"
                else "no assertion in it"
            )
            findings.append(
                f"{rel}: a {kind} test file with {why}. A test that calls the "
                f"code and states nothing about the answer passes forever."
            )

    total = sum(counted.values())
    if not total:
        print(f"NOTHING WAS READ: no test file under {ROOT}", file=sys.stderr)
        return 2

    if findings:
        print("test file(s) with no statement of what the result should be:\n")
        for f in findings:
            print(f"  - {f}")
        print(
            "\n  Add the assertion, or - if the oracle is real but not a literal "
            "one, like a\n  test whose claim is that nothing panics - name the file "
            "in KNOWN with the\n  reason. Whether an assertion is a GOOD one is not "
            "this check's business."
        )
        return 1

    print(
        f"{total} test file(s) read ({counted['rust']} rust, {counted['vitest']} "
        f"vitest, {counted['control']} control), each carrying an assertion; "
        f"{len(excused)} excused by name. It reads whether an assertion is THERE, "
        f"never whether it is right - an assertion that encodes today's bug passes "
        f"this and defends the bug."
    )
    for e in excused:
        print(f"  excused  {e}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
