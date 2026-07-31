# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that every daemon binary is either smoked or skipped with a reason.

`smoke-daemons.sh` starts the daemons that can run unattended and checks each
binds the socket it claims. What it cannot do is notice a daemon nobody added to
it: a new one is simply absent, and the run still says OK.

That is not hypothetical. The script's header used to carry its exclusions in
prose and named eight of the twenty-four it actually excluded, while asserting
that skipping things silently is worse than not testing at all. The list was
right about the principle and wrong about itself.

So the classification is data now, and this compares it to the tree. Every crate
under `daemons/` (plus `store-backend`) with a `src/main.rs` produces a binary,
and each must appear in the smoke's `DAEMONS` or in its `SKIPPED` list with a
reason. Adding a daemon then means deciding which it is, rather than defaulting
to untested-and-unmentioned.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
SMOKE = ROOT / "dev/scripts/smoke-daemons.sh"

BIN_SECTION = re.compile(r"\[\[bin\]\](.*?)(?=\n\[|\Z)", re.S)
NAME = re.compile(r'^\s*name\s*=\s*"([^"]+)"', re.M)


def binaries() -> dict[str, str]:
    """Binary name to the crate that builds it, for every daemon with a main.rs."""
    out: dict[str, str] = {}
    roots = sorted(ROOT.glob("daemons/*/")) + sorted(ROOT.glob("daemons/*/*/"))
    roots.append(ROOT / "store-backend")
    for d in roots:
        manifest = d / "Cargo.toml"
        if not manifest.is_file() or not (d / "src/main.rs").is_file():
            continue
        text = manifest.read_text()
        # An explicit `[[bin]] name` wins: several crates build a binary named
        # nothing like their package (knowledge builds `arlen-graph-daemon`).
        named = [m.group(1) for sec in BIN_SECTION.findall(text) for m in [NAME.search(sec)] if m]
        pkg = NAME.search(text)
        if not named and not pkg:
            sys.exit(f"{manifest} declares no name; the check needs updating")
        out[named[0] if named else pkg.group(1)] = str(d.relative_to(ROOT))
    if not out:
        sys.exit("found no daemon binaries at all; the check needs updating")
    return out


def listed(array: str) -> dict[str, str]:
    """Entries of a `NAME=( "a|b" ... )` array in the smoke script, first field to rest."""
    text = SMOKE.read_text()
    m = re.search(rf"^{array}=\((.*?)^\)", text, re.S | re.M)
    if not m:
        sys.exit(f"could not find the {array} array in {SMOKE.name}; the check needs updating")
    out: dict[str, str] = {}
    for entry in re.findall(r'"([^"]+)"', m.group(1)):
        name, _, rest = entry.partition("|")
        out[name] = rest
    return out


def main() -> int:
    built = binaries()
    smoked = listed("DAEMONS")
    skipped = listed("SKIPPED")

    problems: list[str] = []
    for name, crate in sorted(built.items()):
        if name not in smoked and name not in skipped:
            problems.append(
                f"{name} ({crate}) is neither started by the smoke nor listed as skipped. "
                "Add it to DAEMONS with the socket it binds, or to SKIPPED with the reason it cannot run."
            )
    for name in sorted(set(smoked) & set(skipped)):
        problems.append(f"{name} is both started and skipped; the smoke will run it and the list lies")
    for name in sorted((set(smoked) | set(skipped)) - set(built)):
        problems.append(f"{name} is listed but builds no binary any more; delete the entry")
    for name, reason in sorted(skipped.items()):
        if not reason.strip():
            problems.append(f"{name} is skipped with no reason given")

    if problems:
        print("the daemon smoke does not account for every daemon:\n")
        for p in problems:
            print(f"  - {p}")
        return 1

    print(
        f"{len(built)} daemon binarie(s): {len(smoked)} smoked, "
        f"{len(skipped)} skipped, each with a reason"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
