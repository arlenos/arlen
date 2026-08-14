#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""No two crates produce a binary with the same name.

A name is how everything outside the tree refers to a program: a `.desktop`
`Exec=`, a keybinding, a systemd `ExecStart=`, a `Command::new` in another
daemon. Two crates producing the same name means that reference resolves to
whichever one the image installed last, and nothing anywhere says which that is.
The failure has no symptom until someone runs it and gets the other program.

This is the same family as `check-socket-servers` and `check-spawned-binaries`:
the pieces are all correct on their own, and the assembly is where the meaning
goes missing. Unlike those two it needs no hand-kept table - a Cargo manifest
states its binary names outright, so both sides are derived and the check cannot
drift from the tree.

Found by following `arlen-screenshot`, which is the Tauri app AND a CLI in
`sdk/screen-capture`. Neither is installed yet, so it is a trap set for whoever
writes that install phase rather than a live break.

Two shapes count as producing a binary:

    explicit   a `[[bin]]` section with a `name`
    implicit   a package with `src/main.rs` and no `[[bin]]` at all, which cargo
               names after the package

A `[[bin]]` whose name equals its own package name is one binary, not two.
"""

import re
import sys
from pathlib import Path

# An explicit root exists so the control can run this against a synthetic tree of
# crates rather than the real one. `OWN_TREE` records which it is, because the
# carried list below describes THIS repository: against someone else's directory
# an entry naming a crate that is not there is expected, not a stale entry.
OWN_TREE = len(sys.argv) <= 1
REPO = Path(__file__).resolve().parents[2] if OWN_TREE else Path(sys.argv[1]).resolve()

SKIP = ("/target/", "node_modules", "mkosi.builddir", "/.git/")

# Collisions carried with a reason. An entry is a question for whoever owns the
# naming, not a permission to leave it: the rule is one name one program, and
# this records which decision is outstanding.
KNOWN: dict[str, str] = {
    "arlen-screenshot": (
        "two different programs: the Tauri annotate app (apps/screenshot/src-tauri) "
        "and the capture CLI (sdk/screen-capture/src/bin). Neither is installed by "
        "the image yet, so nothing is broken today - but a `.desktop` or a "
        "keybinding pointing at `arlen-screenshot` is already ambiguous, and the "
        "app's own frontend calls only `capture_output`, so the CLI is where "
        "region and window capture actually live. Adjudicate which one owns the "
        "name; the other wants a suffix (`-capture`, `-gui`) before either ships."
    ),
}


def binaries(repo: Path) -> dict[str, set[str]]:
    """binary name -> the crate directories that produce it."""
    out: dict[str, set[str]] = {}
    for manifest in sorted(repo.rglob("Cargo.toml")):
        if any(s in str(manifest) for s in SKIP):
            continue
        text = manifest.read_text(encoding="utf-8", errors="replace")
        crate = str(manifest.parent.relative_to(repo))

        # The package name, read from inside `[package]` only. A bare search for
        # `name =` finds the first one in the file, which for a workspace root or
        # a manifest whose `[lib]` comes first is not the package at all.
        pkg = None
        pm = re.search(r"^\s*\[package\]\s*$", text, re.M)
        if pm:
            nm = re.search(r'^\s*name\s*=\s*"([^"]+)"', text[pm.end():], re.M)
            if nm:
                pkg = nm.group(1)

        names = set()
        for section in re.finditer(r"\[\[bin\]\](.*?)(?=\n\s*\[|\Z)", text, re.S):
            nm = re.search(r'^\s*name\s*=\s*"([^"]+)"', section.group(1), re.M)
            if nm:
                names.add(nm.group(1))
        # Cargo's implicit binary. Only when no `[[bin]]` is declared: once one
        # is, the file is explicit about what it builds.
        if not names and pkg and (manifest.parent / "src" / "main.rs").is_file():
            names.add(pkg)

        for n in names:
            out.setdefault(n, set()).add(crate)
    return out


def main() -> int:
    found = binaries(REPO)
    if not found:
        print(f"NOTHING WAS READ: no Cargo manifests under {REPO}", file=sys.stderr)
        return 2

    problems: list[str] = []
    carried: list[str] = []

    for name, crates in sorted(found.items()):
        if len(crates) < 2:
            continue
        where = ", ".join(sorted(crates))
        if name in KNOWN:
            carried.append(f"{name} ({where}): {KNOWN[name]}")
            continue
        problems.append(
            f"{name}: built by {where}.\n"
            f"    A `.desktop`, a keybinding or another daemon's `Command::new` "
            f"naming it gets whichever was installed last. Rename one, or say "
            f"here which program owns the name."
        )

    for name in sorted(KNOWN) if OWN_TREE else []:
        if name not in found:
            problems.append(
                f"{name}: carried as a collision, and no crate builds it any more. "
                f"Drop the entry - a carried problem that resolved itself reads as "
                f"coverage."
            )
        elif len(found[name]) < 2:
            problems.append(
                f"{name}: carried as a collision, but only {next(iter(found[name]))} "
                f"builds it now. Drop the entry; the decision was taken."
            )

    if carried:
        print("carried, with a reason (see KNOWN):")
        for line in carried:
            print(f"  {line}")
        print()

    if problems:
        print("two crates claim one binary name:")
        for p in problems:
            print(f"  {p}")
        return 1

    print(f"OK: {len(found)} binary name(s), each built by exactly one crate")
    return 0


if __name__ == "__main__":
    sys.exit(main())
