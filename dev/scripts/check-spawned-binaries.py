#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A first-party program the tree spawns by name has to be one the image installs.

`Command::new("arlen-settings")` is a string. Whether that binary exists in the
booted image is decided in `dev/mkosi/mkosi.build.d`, by a different file, in a
different language. **Nothing links the two**, so a spawn of an absent binary
compiles, passes every test, and fails at the click with `No such file or
directory` - on the machine, in front of the user.

Two of those turned up within an hour of each other on 11 Aug, which is what this
file is for.

`arlen-run` was the first. The whole stamped-identity chain keys on it - the
launcher registers a confined child's pidfd at the broker, and the broker admits
exactly one registrar id - and no build step compiled it. Every piece built, the
producer absent, so Tier 1 could never fire for anything.

`arlen-settings` is the second, found by asking the same question of every spawn
site rather than of one: four call sites in three shipped apps run it (the shell's
quick actions and settings provider, the harness's AI management, the knowledge
app's settings link), the config-broker admits `dev.arlen.settings` as its ONLY
config writer, and the image has no Settings build step at all. It is absent
rather than blocked - `cargo check` on `apps/settings/src-tauri` is clean.

**Only `arlen-*`, and the exclusion is the whole reason this is usable.** The tree
also spawns `pactl`, `nmcli`, `wpctl`, `flatpak`, `zenity`, `wf-recorder`,
`udisksctl`, `xdg-open` and a dozen more, and the image's package list contains
none of them. That is a real question - the shell's network, audio and power
surfaces shell out to tools the image does not install - but it is a question
about which PACKAGES an image installs, which is a build decision this repo should
not hold an opinion about in a script. Reported to the planner instead. Rule of
thumb the rest of `dev/scripts` follows: a check that reports fifteen things
somebody already decided is a check nobody reads.

So the scope here is narrow on purpose: **our own binaries, where "should it be
there" has only one sensible answer** - the tree spawns it, so it has to be there.
"""

import re
import sys
from pathlib import Path

ROOT = (
    Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
)

SPAWN = re.compile(r'Command::new\(\s*"(arlen-[A-Za-z0-9._-]+)"')
# `install -Dm755 ... "$DESTDIR/usr/..."` and the `/usr/bin` symlinks beside them.
INSTALL = re.compile(r'"\$DESTDIR(/usr/[A-Za-z0-9_./-]+)"')
SYMLINK = re.compile(r'ln -sf\s+"?([A-Za-z0-9_./-]+)"?\s+"\$DESTDIR(/usr/[A-Za-z0-9_./-]+)"')

# binary -> why it is not installed, and what that is waiting on. A bare name
# would let the exception outlive the reason it was granted for.
KNOWN = {
}


def spawned(root: Path) -> tuple[dict[str, list[str]], int]:
    """`arlen-*` program name -> the files that spawn it, and how many were read.

    The count is returned rather than derived, because zero spawn sites and zero
    FILES are different facts: the first is a tree that spawns nothing, the second
    is a walk that reached nothing.
    """
    out: dict[str, list[str]] = {}
    scanned = 0
    for path in sorted(root.rglob("*.rs")):
        sp = str(path)
        if "/target/" in sp or "mkosi.builddir" in sp:
            continue
        scanned += 1
        text = path.read_text(encoding="utf-8", errors="replace")
        for name in set(SPAWN.findall(text)):
            out.setdefault(name, []).append(str(path.relative_to(root)))
    return out, scanned


def installed(root: Path) -> set[str]:
    """Every basename the image build steps put somewhere under `/usr`."""
    names: set[str] = set()
    steps = root / "dev/mkosi/mkosi.build.d"
    if steps.is_dir():
        for step in sorted(steps.iterdir()):
            if not step.is_file():
                continue
            text = step.read_text(encoding="utf-8", errors="replace")
            for dest in INSTALL.findall(text):
                names.add(dest.rsplit("/", 1)[1])
            for _, link in SYMLINK.findall(text):
                names.add(link.rsplit("/", 1)[1])
    # Anything staged verbatim counts as installed too.
    extra = root / "dev/mkosi/mkosi.extra"
    if extra.is_dir():
        for p in extra.rglob("*"):
            if p.is_file() and not p.is_symlink():
                names.add(p.name)
    return names


def main() -> int:
    have = installed(ROOT)
    sites, scanned = spawned(ROOT)
    # Measured 27 August against an empty directory: this printed "OK: 0
    # first-party program(s) spawned by name" and exited 0, which is the
    # all-clear a renamed directory would earn.
    if scanned == 0:
        print(
            "NOTHING WAS READ: no Rust source under this tree, so no spawn site "
            "was examined. The layout moved or this check is pointed at the wrong "
            "root.",
            file=sys.stderr,
        )
        return 2
    problems: list[str] = []
    carried: list[str] = []

    for name, files in sorted(sites.items()):
        if name in have:
            continue
        where = ", ".join(sorted(files)[:3])
        if name in KNOWN:
            carried.append(f"{name}: {KNOWN[name]}")
            continue
        problems.append(f"{name} (spawned by {where}) is installed by no build step")

    # An entry that has come true, both ways it can. `arlen-run` left this list
    # by gaining `08r-arlen-run.sh.chroot`, and nothing here would have said so -
    # the loop above skips an installed binary before it ever looks at KNOWN, and
    # it only walks spawn SITES, so an entry whose last caller is gone is never
    # examined at all. Each entry is a sentence about the tree ("has no build step
    # at all", "the harness has no build step either"), and a sentence that has
    # stopped being true reads as work somebody still owes. The same rot was found
    # in `check-invoke-scope.py` on 12 Aug, where two acknowledged calls had both
    # been fixed months apart and the list still described them as broken.
    for name in sorted(KNOWN):
        if name in have:
            problems.append(
                f"{name} is carried as deliberately absent, but a build step now "
                f"installs it. Drop the entry; its reason has been overtaken."
            )
        elif name not in sites:
            problems.append(
                f"{name} is carried as deliberately absent, but nothing spawns it "
                f"any more. Drop the entry; there is no call site left to fail."
            )

    if problems:
        print("programs the tree spawns that the image does not ship:\n")
        for p in problems:
            print(f"  {p}")
        print("\n  A bare-name spawn resolves on PATH at runtime, so this fails at the")
        print("  click rather than at build. Add the build step, or list it in KNOWN")
        print("  with the reason it is deliberately absent.")
        return 1

    shipped = len(sites) - len(carried)
    print(
        f"OK: {len(sites)} first-party program(s) spawned by name, "
        f"{shipped} shipped, {len(carried)} carried"
    )
    if carried:
        print(f"  Carried ({len(carried)}) - known absent, with the reason:")
        for c in carried:
            print(f"    {c}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
