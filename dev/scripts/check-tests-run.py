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

The same invariant has a second rung, one layer down, and 15 September found two
live instances of it: a Rust function inside `#[cfg(test)] mod tests`, full of
assertions, carrying no `#[test]` attribute. Nothing runs it. **And nothing says
so**, which is the whole reason it survives - `cargo test` reports what it ran and
is silent about what it did not, so the only tool that ever mentions such a
function is clippy, calling it dead code, and only when nothing else in the file
happens to reference it. `apps/files/core` had one sitting between two neighbours
that both carry the attribute; it had never run once.

The rung is deliberately narrow, because a `mod tests` block also holds helpers
and a scan cannot tell a helper from a test by name (a tree-wide sweep that day
returned three candidates and two were helpers). So it asks for the actual
invariant - an assertion nobody runs - and clears everything else by
construction: the function must contain an `assert`, and nothing in its own file
may call it. A helper is called; that is what makes it a helper.

And a third, the cheapest of the three: an `#[ignore]` says WHY. Sixty places in
this tree write `#[ignore = "needs a pty"]` or `"needs bwrap and unprivileged
user namespaces"`, which is what lets a reader tell a test that is waiting for a
machine from one that was switched off and forgotten. Six wrote the bare
attribute. The bare form is the one that rots: nobody can tell, a year later,
whether it is still true.

Run: dev/scripts/check-tests-run.py [tree]
"""

import json
import re
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


#: A function declaration, with whatever qualifiers Rust allows before `fn`.
FN = re.compile(
    r"^(\s*)(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?fn\s+([A-Za-z_]\w*)"
)
#: An attribute line, the thing that would make the function run.
ATTR = re.compile(r"^\s*#\[")
#: The opening line of a module block.
MOD = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*\{")
#: Directories that hold code nobody wrote here.
SKIP_PARTS = {"target", "node_modules", ".git", "vendor"}


def test_module_ranges(lines: list[str]) -> list[tuple[int, int]]:
    """Line ranges of every `#[cfg(test)] mod ... { ... }` block in a file."""
    ranges = []
    for i, line in enumerate(lines):
        if "#[cfg(test)]" not in line:
            continue
        j = i + 1
        while j < len(lines) and (ATTR.match(lines[j]) or not lines[j].strip()):
            j += 1
        if j >= len(lines) or not MOD.match(lines[j]):
            continue
        depth, k = 0, j
        while k < len(lines):
            depth += lines[k].count("{") - lines[k].count("}")
            if depth <= 0 and k > j:
                break
            k += 1
        ranges.append((j, min(k, len(lines) - 1)))
    return ranges


def orphaned_assertions(path: Path) -> list[tuple[int, str]]:
    """Functions in a test module that assert, carry no attribute and nobody calls."""
    text = path.read_text(encoding="utf-8", errors="replace")
    lines = text.split("\n")
    found = []
    for start, end in test_module_ranges(lines):
        i = start + 1
        while i <= end:
            declaration = FN.match(lines[i])
            if not declaration:
                i += 1
                continue
            name = declaration.group(2)
            attrs, j = [], i - 1
            while j > start and (
                ATTR.match(lines[j]) or lines[j].lstrip().startswith("//") or not lines[j].strip()
            ):
                if ATTR.match(lines[j]):
                    attrs.append(lines[j])
                j -= 1
            # `test]` covers `#[test]`, `test(` covers `#[tokio::test(flavor = ..)]`
            # and every harness attribute that reads the same way.
            runs = any("test]" in a or "test(" in a for a in attrs)
            depth, k, body = 0, i, []
            while k <= end:
                body.append(lines[k])
                depth += lines[k].count("{") - lines[k].count("}")
                if depth <= 0 and "{" in "".join(body):
                    break
                k += 1
            if not runs and "assert" in "\n".join(body):
                # A helper is called. That is what makes it a helper, and it is the
                # one signal that separates the two without reading intent.
                if len(re.findall(r"\b" + re.escape(name) + r"\b", text)) == 1:
                    found.append((i + 1, name))
            i = k + 1
    return found


#: A bare `#[ignore]` - no reason given. The `= "..."` form is what this asks for.
BARE_IGNORE = re.compile(r"^\s*#\[ignore\]\s*$", re.M)


def unexplained_ignores(path: Path) -> list[tuple[int, str]]:
    """Every `#[ignore]` with no reason, and the function it sits above."""
    lines = path.read_text(encoding="utf-8", errors="replace").split("\n")
    found = []
    for i, line in enumerate(lines):
        if not BARE_IGNORE.match(line):
            continue
        name = "?"
        for k in range(i + 1, min(i + 4, len(lines))):
            m = FN.match(lines[k])
            if m:
                name = m.group(2)
                break
        found.append((i + 1, name))
    return found


def rust_sources(root: Path) -> list[Path]:
    """Every Rust file in the tree that somebody here wrote."""
    return [p for p in root.rglob("*.rs") if not SKIP_PARTS & set(p.parts)]


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

    sources = rust_sources(root)
    orphans: list[str] = []
    silent_ignores: list[str] = []
    for src in sources:
        for line, name in orphaned_assertions(src):
            orphans.append(f"{src.relative_to(root)}:{line}: `{name}` asserts and has no `#[test]`")
        for line, name in unexplained_ignores(src):
            silent_ignores.append(f"{src.relative_to(root)}:{line}: `{name}` is ignored and does not say why")

    if checked == 0 and not sources:
        print("NOTHING WAS READ: no component with test files and no Rust source", file=sys.stderr)
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

    if orphans:
        print("assertions that nothing runs:", file=sys.stderr)
        for o in orphans:
            print(f"  {o}", file=sys.stderr)
        print(
            "\nAdd the attribute, then run it - the assertion has never been checked,"
            "\nso it may be stating something that stopped being true. If it is a helper"
            "\nrather than a test, it is one nothing calls, and it should go.",
            file=sys.stderr,
        )

    if silent_ignores:
        print("tests that do not run and do not say why:", file=sys.stderr)
        for i in silent_ignores:
            print(f"  {i}", file=sys.stderr)
        print(
            '\nWrite the reason into the attribute - `#[ignore = "needs a pty"]`,'
            "\nthe form sixty other tests here use. It is what lets the next reader tell"
            "\na test waiting for a machine from one somebody switched off.",
            file=sys.stderr,
        )

    if silent or orphans or silent_ignores:
        return 1

    print(
        f"check-tests-run: {checked} component(s) with tests declare a script,"
        f" {len(sources)} Rust file(s) hold no assertion nothing runs and no"
        " unexplained `#[ignore]`"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
