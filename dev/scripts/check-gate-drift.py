# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that CI and the justfile still run the same commands, not just the same crates.

`check-crate-coverage.py` compares the two build LISTS. That is the drift that
used to happen, and it no longer does. What it cannot see is the two gates
agreeing on which crates to build and then building them differently, which has
happened three separate times: nextest exiting 4 on a test-free crate where CI's
`cargo test` passes, `cargo test --doc` exiting 101 on a binary-only crate, and
the frontend test runner needing a vitest-only flag that CI branched on and the
justfile did not. Each one looked like a broken crate rather than a broken gate.

Two things are still duplicated by hand and would drift the same way:

  1. The crates whose tests must run serially. Both files carry the same `case`
     arm. Add a crate to one only and you get either a CI flake nobody can
     reproduce locally, or a local flake CI never sees - and shared-state test
     races are the hardest failures to attribute.

  2. `CXXFLAGS`. lbug's vendored thrift will not compile without it, and it is
     set in three places for three audiences: `.cargo/config.toml` for a bare
     `cargo`, `dev/justfile` for `just`, `ci.yml` for the runner. If they drift,
     whichever entry point has the stale value stops being able to build the
     knowledge daemon at all.

Both are compared as values rather than pattern-matched loosely, so a change to
either has to be made in every place before this passes.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]

CI = ".github/workflows/ci.yml"
JUSTFILE = "dev/justfile"
CARGO_CONFIG = ".cargo/config.toml"

# `<crates>) extra="... --test-threads=1"`, in either file's `case` block.
SERIAL = re.compile(r"^\s*([\w/|.-]+)\)\s*extra=\"[^\"]*--test-threads=1\"", re.M)

# `CXXFLAGS = "..."` (toml), `export CXXFLAGS := "..."` (just), `CXXFLAGS: "..."` (yaml).
CXXFLAGS = re.compile(r"^[^#\n]*\bCXXFLAGS\s*(?::=|=|:)\s*\"([^\"]*)\"", re.M)


def read(rel: str) -> str:
    return (ROOT / rel).read_text()


def serial_crates(rel: str) -> set[str]:
    hits = SERIAL.findall(read(rel))
    if not hits:
        sys.exit(f"no serial-test case arm found in {rel}; the check needs updating")
    return {c for arm in hits for c in arm.split("|")}


def cxxflags(rel: str) -> str:
    hits = CXXFLAGS.findall(read(rel))
    if not hits:
        sys.exit(f"no CXXFLAGS assignment found in {rel}; the check needs updating")
    if len(set(hits)) > 1:
        sys.exit(f"{rel} sets CXXFLAGS to more than one value: {sorted(set(hits))}")
    return hits[0]


def main() -> int:
    problems: list[str] = []

    ci_serial = serial_crates(CI)
    just_serial = serial_crates(JUSTFILE)
    for crate in sorted(ci_serial - just_serial):
        problems.append(f"{crate} runs serially in CI but in parallel under `just test`")
    for crate in sorted(just_serial - ci_serial):
        problems.append(f"{crate} runs serially under `just test` but in parallel in CI")

    flags = {rel: cxxflags(rel) for rel in (CARGO_CONFIG, JUSTFILE, CI)}
    if len(set(flags.values())) > 1:
        where = "; ".join(f"{rel} has {v!r}" for rel, v in sorted(flags.items()))
        problems.append(f"CXXFLAGS disagrees: {where}")

    if problems:
        print("the gates run different commands:\n")
        for p in problems:
            print(f"  - {p}")
        print("\nthe same crate has to be built the same way by CI and by `just`")
        return 1

    print(
        f"{len(ci_serial)} serial crate(s) agree, CXXFLAGS agrees across "
        f"{len(flags)} file(s): {next(iter(flags.values()))!r}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
