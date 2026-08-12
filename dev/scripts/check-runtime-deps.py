#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Every external binary our code calls is listed, and the listed ones are shipped.

Our runtime dependencies were implicit. We knew what we `Cargo.toml`; nobody had
written down what the shipped system must PROVIDE for our own code to run, and it
cost three findings in one day - `user-dirs.dirs` never written, `(unset)`
swallowing the keyboard layout, polkitd absent so the power menu could not
authorise a suspend. Each found by a different accident, none by a check.

Enumerating it properly was worse than expected: twenty-one of thirty-three
external binaries were absent from the image, including every one behind
networking, audio, the clipboard and opening a file. A missing binary is not a
loud failure - it is `No such file or directory` at spawn, inside a feature nobody
had exercised on the appliance.

Two halves, and the second is the one that keeps it honest:

    completeness   every `Command::new("...")` in the tree appears in
                   `runtime-deps.tsv`. A new shell-out has to be classified, which
                   is the moment somebody decides whether the image ships it.
    shipping       every entry marked `ships` names a package that mkosi.conf
                   actually lists. Removing `polkitd` from Packages turns this red,
                   which is the check the whole exercise was asked for.

It does NOT verify that `absent` entries are absent. That would forbid the image
ever gaining a package without a code change, and the list's job is to record the
decision, not to freeze it.
"""

import pathlib
import re
import sys

ROOT = (
    pathlib.Path(sys.argv[1]).resolve()
    if len(sys.argv) > 1
    else pathlib.Path(__file__).resolve().parents[2]
)
TSV = pathlib.Path(__file__).resolve().parent / "runtime-deps.tsv"
MKOSI = ROOT / "dev/mkosi/mkosi.conf"
SCAN = ("apps", "daemons", "sdk", "ai", "contracts")

VALID = {"ships", "own", "base", "absent"}


def listed() -> dict[str, tuple[str, str]]:
    """binary -> (package, status) from the TSV."""
    out: dict[str, tuple[str, str]] = {}
    for line in TSV.read_text().splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        parts = line.split("\t")
        if len(parts) < 4:
            sys.exit(f"runtime-deps.tsv: malformed line: {line!r}")
        what, package, _who, status = parts[0], parts[1], parts[2], parts[3]
        if status not in VALID:
            sys.exit(f"runtime-deps.tsv: {what}: unknown status {status!r}")
        out[what] = (package, status)
    return out


def called(root: pathlib.Path) -> set[str]:
    """Every literal binary name passed to `Command::new`."""
    found: set[str] = set()
    pattern = re.compile(r'Command::new\(\s*"([a-z0-9_.+-]+)"\s*\)')
    for sub in SCAN:
        base = root / sub
        if not base.is_dir():
            continue
        for path in base.rglob("*.rs"):
            if {"target", "node_modules"} & set(path.parts):
                continue
            found.update(pattern.findall(path.read_text(encoding="utf-8", errors="replace")))
    return found


def packages(conf: pathlib.Path) -> set[str]:
    """Package names inside mkosi.conf's `Packages=` block."""
    if not conf.is_file():
        return set()
    text = conf.read_text()
    m = re.search(r"^Packages=\n(.*?)(?=^\w+=|\Z)", text, re.M | re.S)
    if not m:
        return set()
    out = set()
    for line in m.group(1).splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            out.add(line)
    return out


def main() -> int:
    if not TSV.is_file():
        print(f"NOTHING WAS READ: no {TSV}", file=sys.stderr)
        return 2
    entries = listed()
    if not entries:
        print(f"NOTHING WAS READ: {TSV} lists nothing", file=sys.stderr)
        return 2
    spawned = called(ROOT)
    if not spawned:
        print(f"NOTHING WAS READ: no Command::new under {ROOT}", file=sys.stderr)
        return 2

    problems: list[str] = []

    # The completeness half compares the list against THIS repo, so it only means
    # anything against this repo. Run against a fixture it would report every
    # binary the fixture happens not to call as a stale entry - the same category
    # error as check-log-filters' excuse list, found the same way, by a control
    # that could not construct a passing tree. The shipping half below works
    # anywhere, which is what lets the control plant a missing package.
    own_tree = len(sys.argv) <= 1
    for binary in sorted(spawned - set(entries)):
        problems.append(
            f"{binary} is spawned by our code and is not in runtime-deps.tsv. Add it "
            f"with the package that provides it and whether the image ships it - an "
            f"unlisted shell-out is one that fails at spawn on a machine without it."
        )
    if own_tree:
        for binary in sorted(set(entries) - spawned):
            problems.append(
                f"{binary} is listed in runtime-deps.tsv and nothing spawns it any "
                f"more; delete the entry so the list keeps describing the tree."
            )

    named = packages(MKOSI)
    if not named:
        problems.append(
            f"no Packages= block found in {MKOSI.relative_to(ROOT)}; the shipping "
            f"half of this check cannot run and must not pass quietly."
        )
    else:
        for binary, (package, status) in sorted(entries.items()):
            if status != "ships":
                continue
            if package not in named:
                problems.append(
                    f"{binary} is marked `ships` and needs `{package}`, which "
                    f"mkosi.conf does not name. Either add the package or change the "
                    f"entry to `absent` with what that costs."
                )

    if problems:
        print("runtime dependencies:\n")
        for p in problems:
            print(f"  - {p}")
        return 1

    by_status: dict[str, int] = {}
    for _p, status in entries.values():
        by_status[status] = by_status.get(status, 0) + 1
    absent = by_status.get("absent", 0)
    print(
        f"{len(entries)} external binaries listed: "
        + ", ".join(f"{n} {s}" for s, n in sorted(by_status.items()))
    )
    if absent:
        print(
            f"  {absent} deliberately absent - each names the feature that does not "
            f"work on this image, which is a decision on the record rather than a "
            f"failure at spawn"
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
