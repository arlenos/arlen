#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Every bus name a shipped unit declares is covered by the served-object list.

`dev/scripts/served-objects.tsv` says which bus surfaces exist and which object
path each must serve; `probe-served-objects.sh` asks running daemons whether they
do. That list is hand-kept for reasons written in its own header, and hand-kept
lists shrink: a daemon lands, its unit declares a name, nobody adds the pair, and
the sweep stays green while covering one surface less than it did yesterday. The
gate would then be reporting on what somebody remembered rather than on what
ships.

**`BusName=` is a declaration, not a guess.** A unit that carries one is telling
systemd this service owns that name, which makes it the honest source for what
the list must contain. Ten names across the shipped units today.

The check is one-directional: every declared name must appear in the list, not
the reverse. A surface with no packaged unit yet - the AI engine daemon owns two
and ships none - is a real surface worth probing, and demanding a unit for it
would push the list to cover LESS.

A name may be carried as `!exclude <name> <reason>` instead of a pair. That is
for a name this probe genuinely cannot drive - the root-owned system-bus helpers,
an impl-portal backend reached through the frontend - and the reason is read by a
person, which is the difference between an exclusion and a hole.

What this does NOT check: whether the path in a pair is the right one (only a
running daemon knows, which is the probe's job), and whether a name nobody
declared in a unit is missing from the list. The second is the residual: a daemon
with neither a unit nor an entry is invisible here, and no source-side rule can
see a surface that declares itself nowhere.

Usage: check-bus-names-covered.py [repo-root]
"""

import pathlib
import subprocess
import re
import sys
from pathlib import Path

# The tree to scan. An argument so this can be pointed at a fixture and shown
# to fail: a check that only ever runs against a tree that already passes
# cannot demonstrate the defect it exists for (standing rule, 11 Aug).
ROOT = (
    Path(sys.argv[1]).resolve()
    if len(sys.argv) > 1
    else Path(__file__).resolve().parents[2]
)

LIST = "dev/scripts/served-objects.tsv"
BUS_NAME = re.compile(r"^BusName\s*=\s*(\S+)\s*$", re.MULTILINE)


SKIP_DIRS = {"target", "node_modules", ".git", "mkosi.builddir", "mkosi.cache"}


def shipped_units(root: Path) -> list[Path]:
    """Units THIS tree ships, using `check-shipped-units.py`'s own definition.

    Two sources, and the scope matters more than it looks. A bare walk for
    `*.service` also finds `dev/mkosi/mkosi.tools`, a vendored distro toolchain
    full of systemd's own units - `org.freedesktop.login1`, `org.rpm.dnf.v0` and a
    dozen more - and demanding our list account for logind would be demanding it
    account for the operating system. It also finds `.wants/` enablement symlinks
    pointing into an image root that does not exist here, which are links rather
    than declarations.
    """
    out = [
        p
        for p in root.glob("**/dist/**/*.service")
        if not SKIP_DIRS & set(p.parts)
    ]
    installed = root / "dev/mkosi/mkosi.extra/usr/lib/systemd"
    for sub in ("system", "user"):
        out.extend(sorted((installed / sub).glob("*.service")))
    return sorted(set(out))


def declared(root: Path) -> tuple[dict[str, list[str]], list[str]]:
    """`BusName=` -> the units declaring it, plus units that could not be read."""
    out: dict[str, list[str]] = {}
    unreadable: list[str] = []
    for path in shipped_units(root):
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError as e:
            # Reported, not swallowed: a unit of ours we cannot read might be
            # declaring a name, and silence there is the failure this whole
            # family of checks exists to stop.
            unreadable.append(f"{path.relative_to(root)}: {e.strerror}")
            continue
        for m in BUS_NAME.finditer(text):
            out.setdefault(m.group(1), []).append(str(path.relative_to(root)))
    return out, unreadable


def covered(list_path: Path) -> set[str]:
    """The bus names the served-object list accounts for, paired or excluded."""
    names: set[str] = set()
    for line in list_path.read_text(encoding="utf-8", errors="replace").splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        fields = line.split("\t")
        if len(fields) < 3 or not fields[2].strip():
            continue
        # Both forms carry the bus name in the middle column: `binary name path`
        # and `!exclude name reason`. An exclusion with an empty reason is not a
        # reason, so the third column must be non-empty either way.
        names.add(fields[1].strip())
    return names


# Dangling callers carried with a reason, because resolving them is the planner's
# call. An entry is a question, not a permission.
CARRIED = {
    "org.arlen.Accounts1": (
        "DECIDED 13 Aug: do NOT ship it. Accounts, connections and transfers are "
        "three daemons and a subsystem, and putting them in front of people looking "
        "functional while incomplete is worse than the absence. The caller changed "
        "instead - the file manager now returns `unavailable` rather than an empty "
        "list, so a missing subsystem cannot read as 'you have no accounts'. This "
        "entry waits for the subsystem to be finished, not for a gate."
    ),
    "org.arlen.Connections1": (
        "DECIDED 13 Aug, the same sitting as Accounts1 above: the ai-proxy carries a "
        "full client proxy for the Connections egress-delivery surface "
        "(`connections_client.rs`, default_service set), and the connections daemon "
        "does not ship. The cost here is that the proxy's delivery path cannot "
        "complete on the image. Dated explicitly rather than inheriting 'same "
        "adjudication' from its neighbour: an exception whose date is a pointer at "
        "another entry stops having one the moment that entry is edited, and the "
        "rule these are kept under is that each names its own."
    ),
}


def owners(list_path: Path) -> dict[str, str]:
    """bus name -> the binary that owns it, from the paired rows only.

    Exclusions are deliberately absent: `!exclude` says a name is not probed here,
    never that something serves it. `org.arlen.Transfer1` is excluded precisely
    because NOBODY takes it - reading exclusions as ownership would turn the one
    row that documents that into a claim of the opposite.
    """
    out: dict[str, str] = {}
    for line in list_path.read_text(encoding="utf-8", errors="replace").splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        fields = line.split("\t")
        if len(fields) < 3 or fields[0].strip() == "!exclude":
            continue
        out[fields[1].strip()] = fields[0].strip()
    return out


def shipped_binaries(root: Path) -> set[str]:
    """Binary names the image build installs - the same derivation the socket gate
    uses, kept here rather than shared because a two-line import across gates buys
    less than each one being readable on its own."""
    names: set[str] = set()
    phases = root / "dev/mkosi/mkosi.build.d"
    for phase in sorted(phases.glob("*.chroot")) if phases.is_dir() else []:
        for dest in re.findall(r'"\$DESTDIR(/[^"]*)"', phase.read_text(encoding="utf-8")):
            if "/bin/" in dest or "/libexec/" in dest:
                names.add(dest.rsplit("/", 1)[-1])
    for sub in ("usr/bin", "usr/lib/arlen/libexec"):
        d = root / "dev/mkosi/mkosi.extra" / sub
        if d.is_dir():
            names.update(p.name for p in d.iterdir() if p.is_file())
    return names


def callers(root: Path, shipped: set[str], known: set[str]) -> dict[str, set[str]]:
    """Names a SHIPPED component calls -> the crates calling them.

    Only names the list already knows, which is what keeps this sound: a scan by
    string SHAPE over-collects badly (this tree has app ids and graph namespaces of
    the same form), and the list's own header says so. So this asks a narrower
    question - of the names we KNOW are bus surfaces, which does a shipped
    component dial - and leaves discovering new ones to the coverage half above.
    """
    out: dict[str, set[str]] = {}
    grep = subprocess.run(
        ["git", "grep", "-l", "-F", "--", "org.arlen.", "--", "*.rs"],
        cwd=root, capture_output=True, text=True,
    )
    for rel in grep.stdout.split():
        if "/target/" in rel or "mkosi.builddir" in rel or "/tests/" in rel:
            continue
        crate = rel.split("/src/")[0]
        component = "/".join(pathlib.PurePath(crate).parts[:2])
        comp_dir = root / component
        if not comp_dir.is_dir():
            continue
        ships = any(
            set(re.findall(r'^\s*name\s*=\s*"([^"]+)"', m.read_text(encoding="utf-8"), re.M))
            & shipped
            for m in comp_dir.rglob("Cargo.toml") if "/target/" not in str(m)
        )
        if not ships:
            continue
        text = (root / rel).read_text(encoding="utf-8", errors="replace")
        for name in known:
            if f'"{name}"' in text:
                out.setdefault(name, set()).add(crate)
    return out


def main() -> int:
    list_path = ROOT / LIST
    if not list_path.is_file():
        print(f"{LIST} is missing; the served-object list is what this checks against")
        return 1

    units, unreadable = declared(ROOT)
    if not units:
        print("found no unit declaring a BusName; the layout moved and this check went quiet")
        return 1

    have = covered(list_path)
    missing = {n: u for n, u in units.items() if n not in have}

    print(
        f"{len(units)} bus name(s) declared by shipped units, "
        f"{len(units) - len(missing)} covered by {LIST}. "
        f"Paths are not checked here: only a running daemon knows those, which is "
        f"what probe-served-objects.sh is for."
    )

    if unreadable:
        print("\nunit files that could not be read:\n")
        for u in unreadable:
            print(f"  - {u}")

    if missing:
        print("\ndeclared in a unit and absent from the served-object list:\n")
        for name, where in sorted(missing.items()):
            print(f"  - {name}  ({', '.join(sorted(set(where)))})")
        print(
            "\nAdd each as a pair with the object path its callers dial, or as "
            "`!exclude <name> <reason>` if this probe cannot drive it. A name that "
            "is neither is a surface the sweep silently stopped covering."
        )
        return 1

    # The caller's question, which this gate did not ask until 13 Aug: a shipped
    # component may dial a name whose OWNER does not ship, and everything looks
    # fine from here - the name is in the list, the list is complete, and the call
    # fails on the machine every time. The file manager ships and calls
    # `org.arlen.Accounts1.ListAccounts` for its remote-places sidebar; the accounts
    # daemon is built, CI-tested and installed by nothing, so the sidebar is empty
    # by construction rather than because no account is configured.
    ships = shipped_binaries(ROOT)
    owned = owners(list_path)
    dangling = {
        name: crates
        for name, crates in callers(ROOT, ships, set(owned)).items()
        if owned[name] not in ships and name not in CARRIED
    }
    for name in sorted(CARRIED):
        if name in owned and owned[name] in ships:
            print(f"\n{name} is carried as unserved, but {owned[name]} ships now. "
                  f"Drop the entry - a carried gap that closed reads as coverage.")
            return 1
    if dangling:
        print("\na shipped component dials a bus name whose owner is not installed:\n")
        for name, crates in sorted(dangling.items()):
            print(f"  - {name}  owned by {owned[name]}, dialled by {', '.join(sorted(crates))}")
        print(
            "\nShip the owner, or remove the call. This is the same rule as "
            "`check-socket-servers` over the other transport: a caller that dials "
            "forever while nothing answers is a failure dressed as a feature that "
            "is merely not configured."
        )
        return 1
    return 1 if unreadable else 0


if __name__ == "__main__":
    sys.exit(main())
