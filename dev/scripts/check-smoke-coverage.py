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

The desktop apps are held to the same rule against `smoke-apps.sh`, added after
the desktop shell turned out to have been panicking on every start for as long as
its launch socket existed. Nothing read startups, so nobody had to decide whether
an app was covered, and it was silently neither.

What this does NOT check is whether a smoke's assertions are worth anything. It
compares two lists. `smoke-apps.sh` asserting only "no panic, still alive" would
pass this check while proving very little, which is why that script says in its
own header what it does and does not rule in.
"""

import pathlib
import re
import sys

# The tree to scan. An argument so this can be pointed at a fixture and shown
# to fail: a check that only ever runs against a tree that already passes
# cannot demonstrate the defect it exists for (standing rule, 11 Aug).
ROOT = (
    pathlib.Path(sys.argv[1]).resolve()
    if len(sys.argv) > 1
    else pathlib.Path(__file__).resolve().parents[2]
)

# Each smoke script and the crates it is responsible for. The apps were added
# after the desktop shell spent as long as its launch socket existed panicking on
# every start: no check reads a startup, and nothing required anyone to decide
# whether an app was covered, so it was silently neither.
SMOKES = [
    (
        "dev/scripts/smoke-daemons.sh",
        "DAEMONS",
        lambda: sorted(ROOT.glob("daemons/*/")) + sorted(ROOT.glob("daemons/*/*/")) + [ROOT / "store-backend"],
    ),
    (
        "dev/scripts/smoke-apps.sh",
        "APPS",
        lambda: sorted(ROOT.glob("apps/*/src-tauri/")),
    ),
]

BIN_SECTION = re.compile(r"\[\[bin\]\](.*?)(?=\n\[|\Z)", re.S)
NAME = re.compile(r'^\s*name\s*=\s*"([^"]+)"', re.M)


def binaries(roots: list[pathlib.Path]) -> dict[str, str]:
    """Binary name to the crate that builds it, for every crate with a main.rs."""
    out: dict[str, str] = {}
    for d in roots:
        manifest = d / "Cargo.toml"
        if not manifest.is_file() or not (d / "src/main.rs").is_file():
            continue
        text = manifest.read_text()
        # EVERY explicit `[[bin]] name`, not just the first: knowledge builds both
        # `arlen-graph-daemon` and the `arlen-timeline` FUSE helper, and taking
        # only the first meant the second was never demanded to be classified,
        # which is this check failing in the direction that says nothing.
        #
        # A crate with no `[[bin]]` at all falls back to its package name, which
        # is what cargo does for a bare `src/main.rs`.
        named = [m.group(1) for sec in BIN_SECTION.findall(text) for m in [NAME.search(sec)] if m]
        pkg = NAME.search(text)
        if not named and not pkg:
            sys.exit(f"{manifest} declares no name; the check needs updating")
        for name in named or [pkg.group(1)]:
            out[name] = str(d.relative_to(ROOT))
    if not out:
        sys.exit("found no binaries at all; the check needs updating")
    return out


def listed(smoke: pathlib.Path, array: str) -> dict[str, str]:
    """Entries of a `NAME=( "a|b" ... )` array in the smoke script, first field to rest."""
    text = smoke.read_text()
    m = re.search(rf"^{array}=\((.*?)^\)", text, re.S | re.M)
    if not m:
        sys.exit(f"could not find the {array} array in {smoke.name}; the check needs updating")
    out: dict[str, str] = {}
    for entry in re.findall(r'"([^"]+)"', m.group(1)):
        name, _, rest = entry.partition("|")
        out[name] = rest
    return out


def main() -> int:
    problems: list[str] = []
    counted = 0

    for rel, started_array, roots in SMOKES:
        smoke = ROOT / rel
        built = binaries(roots())
        started = listed(smoke, started_array)
        skipped = listed(smoke, "SKIPPED")
        counted += len(built)

        for name, crate in sorted(built.items()):
            if name not in started and name not in skipped:
                problems.append(
                    f"{name} ({crate}) is neither started by {smoke.name} nor listed as skipped. "
                    f"Add it to {started_array}, or to that script's SKIPPED with the reason it cannot run."
                )
        for name in sorted(set(started) & set(skipped)):
            problems.append(
                f"{name} is both started and skipped in {smoke.name}; it will run and the list lies"
            )
        for name in sorted((set(started) | set(skipped)) - set(built)):
            problems.append(f"{name} is listed in {smoke.name} but builds no binary any more; delete the entry")
        for name, reason in sorted(skipped.items()):
            if not reason.strip():
                problems.append(f"{name} is skipped in {smoke.name} with no reason given")

    print(
        f"{counted} binary(s) across {len(SMOKES)} smoke script(s) checked for being started or "
        f"skipped with a reason. Whether the smoke's assertions are strong enough is its own question."
    )
    if problems:
        print("\nnot classified:\n")
        for p in problems:
            print(f"  - {p}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
