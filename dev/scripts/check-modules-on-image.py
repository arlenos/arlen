#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Check that every module in `modules/` reaches the image, under its own id.

WHY THIS EXISTS. On 8 September the module runtime had been shipping for weeks,
its units and permission profile audited, its socket answered - and it hosted
nothing. `modulesd` reads `/usr/share/arlen/modules` and no image build step ever
wrote there, so every keystroke in the launcher asked the runtime for results and
got an empty aggregate. Both modules in the tree built, both were tested against
the real daemon, and neither existed on a booted machine.

That is the sibling of the app-on-image class one layer down, and it hides the
same way: the runtime is healthy, the modules are real, and only the two together
show that they never meet.

A module counts as staged when a `dev/mkosi/mkosi.build.d/*.chroot` step both
names its directory as `modules/<name>` and installs into
`/usr/share/arlen/modules`. Both halves earn their place, for the reason the app
check learned the hard way: a step that only builds would otherwise vouch for a
module nobody ships.

AND THE ID IS CHECKED, which the app check has no equivalent of. The daemon keys a
module's directory by the `[module] id` in its manifest, so a staging line that
installs `core.unicode` for a manifest saying something else produces a directory
the daemon discovers under one name and a module that believes it has another.
The id is read from the manifest rather than from the staging line, so the two
cannot agree by being copied from each other.

Everything else must be in NOT_ON_IMAGE with a reason someone can disagree with.

Shown to fail before being trusted: `dev/scripts/test-check-modules-on-image.mjs`.

Usage: check-modules-on-image.py [repo-root]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

# Module directories that deliberately do not ship, and why. A fixture is the
# clear case: it exists to be instantiated by a test, and putting it on a real
# machine would offer a person a module whose whole purpose is to be refused.
NOT_ON_IMAGE: dict[str, str] = {}

STAGE_DIR = "/usr/share/arlen/modules"


def module_dirs(root: Path) -> list[Path]:
    """Every crate under `modules/`, which is what a module is."""
    return sorted(p.parent for p in root.glob("modules/*/Cargo.toml"))


def manifest_id(module: Path) -> str | None:
    """The id the daemon will key this module's directory by."""
    manifest = module / "manifest.toml"
    if not manifest.is_file():
        return None
    m = re.search(r'^\s*id\s*=\s*"([^"]+)"', manifest.read_text(encoding="utf-8"), re.M)
    return m.group(1) if m else None


def staging_steps(root: Path) -> str:
    """The image build steps, as one blob. Which step stages a module does not
    matter; that one does is the whole question."""
    steps = sorted((root / "dev/mkosi/mkosi.build.d").glob("*.chroot"))
    return "\n".join(p.read_text(encoding="utf-8", errors="replace") for p in steps)


def main() -> int:
    root = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
    steps = staging_steps(root)
    installs = STAGE_DIR in steps

    missing: list[str] = []
    unnamed: list[str] = []
    modules = module_dirs(root)
    for module in modules:
        name = module.name
        rel = f"modules/{name}"
        if name in NOT_ON_IMAGE:
            continue
        if not (installs and rel in steps):
            missing.append(rel)
            continue
        mid = manifest_id(module)
        if mid is None:
            unnamed.append(f"{rel}: no `[module] id` in manifest.toml")
        elif mid not in steps:
            unnamed.append(f"{rel}: manifest id `{mid}` is not the one any step stages")

    if not modules:
        print("check-modules-on-image: no modules found, which means the scan is pointed wrong")
        return 1

    if missing or unnamed:
        print("MODULES THAT NEVER REACH THE IMAGE")
        for rel in missing:
            print(f"  {rel}: no build step names it and installs into {STAGE_DIR}")
        for line in unnamed:
            print(f"  {line}")
        print()
        print("  A module the runtime cannot discover is a module nobody has. Stage it in")
        print("  dev/mkosi/mkosi.build.d/, or add it to NOT_ON_IMAGE with the reason.")
        return 1

    excused = f", {len(NOT_ON_IMAGE)} excused" if NOT_ON_IMAGE else ""
    print(f"check-modules-on-image: {len(modules) - len(NOT_ON_IMAGE)} module(s) staged{excused}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
