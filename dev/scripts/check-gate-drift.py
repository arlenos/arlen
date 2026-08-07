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

The per-crate commands are no longer duplicated: `dev/check-crate.sh` owns them
and both gates call it, so the drift this file used to compare for cannot
happen. What replaces that comparison is keeping the arrangement true, because
it only holds while it is the arrangement:

  1. The serial-test rule lives in `dev/check-crate.sh` and nowhere else. The
     crates whose tests share process-global state must run with
     `--test-threads=1`, and a second copy of that rule growing back in `ci.yml`
     or the justfile is how the two would start disagreeing again. So the check
     is now: the owner names some, and neither caller names any.

  2. Both gates actually go through the script. If one stopped calling it, the
     commands would diverge again with nothing to notice - the script would just
     sit there being correct and unused.

  3. `CXXFLAGS` is still duplicated by hand, and legitimately: lbug's vendored
     thrift will not compile without it, and it is set in three places for three
     audiences - `.cargo/config.toml` for a bare `cargo`, `dev/justfile` for
     `just`, `ci.yml` for the runner. If they drift, whichever entry point has
     the stale value stops being able to build the knowledge daemon at all. It
     is compared as a value, so a change has to be made everywhere before this
     passes.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]

CI = ".github/workflows/ci.yml"
JUSTFILE = "dev/justfile"
CARGO_CONFIG = ".cargo/config.toml"
CHECK_CRATE = "dev/check-crate.sh"

# The per-crate serial rule: a `case` arm mapping crate paths to the flag. Both
# the array form check-crate.sh uses and the string form the two gates used to
# carry, so a copy growing back in the old shape is still seen. Deliberately
# narrower than "mentions --test-threads=1": the nightly and on-host recipes pass
# it to one named test run, which is a different thing from the matrix rule.
SERIAL = re.compile(
    r"^\s*([\w/|.-]+)\)\s*extra=(?:\([^)]*|\"[^\"]*)--test-threads=1", re.M
)

# A per-crate cargo invocation: `cargo check|test ... --manifest-path <something
# holding the crate>`, where the crate comes from the loop or the CI matrix
# rather than being written out. That is the call the script exists to own, so
# either gate making it directly is the drift returning - whatever it happens to
# mention elsewhere in a comment. A one-off recipe naming a literal crate is not
# this, and stays allowed.
PER_CRATE_CARGO = re.compile(
    r"cargo\s+(?:check|test)\b[^\n]*--manifest-path[^\n]*"
    r"(?:\$c\b|\{\{\s*crate\s*\}\}|\$\{\{\s*matrix\.component\s*\}\})"
)

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

    serial = serial_crates(CHECK_CRATE)
    for rel in (CI, JUSTFILE):
        if SERIAL.search(read(rel)):
            problems.append(
                f"{rel} carries its own per-crate serial rule; {CHECK_CRATE} owns that, "
                "and a second copy is how the two gates start disagreeing again"
            )
        if PER_CRATE_CARGO.search(read(rel)):
            problems.append(
                f"{rel} runs cargo per crate itself instead of through {CHECK_CRATE}, "
                "which is how the two gates ran different commands four times"
            )

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
        f"{len(serial)} serial crate(s) declared once in {CHECK_CRATE}, both gates call it, "
        f"CXXFLAGS agrees across {len(flags)} file(s): {next(iter(flags.values()))!r}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
