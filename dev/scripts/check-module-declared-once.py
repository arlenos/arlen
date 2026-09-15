#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A source file belongs to one crate, not two.

WHY THIS EXISTS. `daemons/knowledge` declared its module tree twice: 33 `pub mod`
lines in `lib.rs` and 46 `mod` lines in `main.rs`, 31 of them the same files. Rust
compiles that as TWO crates over one directory, and everything follows from it:

  * every shared module is compiled and tested twice - on the slowest crate in the
    tree that was 624 lib tests and 838 bin tests over largely the same code;
  * `knowledge::graph::GraphHandle` and the binary's `crate::graph::GraphHandle`
    are DIFFERENT TYPES, which is a trap waiting for whoever first tries to pass
    one where the other is expected;
  * dead-code reports lie in both directions. A function whose only caller is in
    the binary reads as never used in the library, and the binary reports every
    lib re-export whose consumer it does not contain - ~480 findings, which is
    what kept `just lint` from reaching the end of its walk.

None of that announces itself. It compiles, the tests pass, and the duplicate
work looks like the crate simply being slow.

WHAT IT CHECKS. For each crate that has both a `src/lib.rs` and a `src/main.rs`:
no module may be declared in both. The fix is always the same shape - the binary
becomes a thin `fn main()` over the library and declares nothing of its own.

A crate with only one of the two is not checked, and a `main.rs` declaring a
module the library does not have is fine: that is a module private to the binary,
compiled once.

Run: dev/scripts/check-module-declared-once.py [tree]
"""

import re
import sys
from pathlib import Path

#: `pub mod name;` in a library root.
LIB_MOD = re.compile(r"^pub mod (\w+)\s*;", re.M)
#: `mod name;` or `pub mod name;` in a binary root.
BIN_MOD = re.compile(r"^(?:pub )?mod (\w+)\s*;", re.M)
#: Directories holding code nobody here wrote.
SKIP = {"target", "node_modules", ".git", "vendor"}


def crate_roots(root: Path) -> list[Path]:
    """Every `src/` that holds BOTH a lib and a bin root."""
    out = []
    for lib in root.rglob("src/lib.rs"):
        if SKIP & set(lib.parts):
            continue
        if (lib.parent / "main.rs").is_file():
            out.append(lib.parent)
    return sorted(out)


def duplicated(src: Path) -> list[str]:
    """Modules declared by both roots, sorted."""
    lib = set(LIB_MOD.findall((src / "lib.rs").read_text(encoding="utf-8", errors="replace")))
    binary = set(BIN_MOD.findall((src / "main.rs").read_text(encoding="utf-8", errors="replace")))
    return sorted(lib & binary)


def main() -> int:
    root = Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()
    roots = crate_roots(root)
    if not roots:
        print(
            "NOTHING WAS READ: no crate in this tree has both a lib and a bin root",
            file=sys.stderr,
        )
        return 2

    findings = []
    for src in roots:
        shared = duplicated(src)
        if shared:
            rel = src.relative_to(root)
            named = ", ".join(shared[:6]) + (" ..." if len(shared) > 6 else "")
            findings.append(f"{rel}: {len(shared)} module(s) declared by both roots: {named}")

    if findings:
        print("modules compiled into two crates at once:", file=sys.stderr)
        for f in findings:
            print(f"  {f}", file=sys.stderr)
        print(
            "\nDeclare each module in `lib.rs` only and let `main.rs` be a thin"
            "\n`fn main()` that uses the library. Two crates over one directory means"
            "\ntwo compilations, two test runs, and two incompatible copies of every"
            "\ntype in them.",
            file=sys.stderr,
        )
        return 1

    print(f"check-module-declared-once: {len(roots)} lib+bin crate(s), no module compiled twice")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
