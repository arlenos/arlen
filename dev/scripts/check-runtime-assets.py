#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Check that every `/usr/share/arlen/...` a component reads is put there by the image.

WHY THIS EXISTS. The terminal app looks for `/usr/share/arlen/terminal/zdotdir` at
startup and falls back to an in-repo path for `cargo tauri dev`. Nothing installed the
first one, so on a real machine neither existed, the lookup did nothing, and zsh started
with its ordinary config - which means the block-mode terminal, the whole point of that
app, had nothing emitting the OSC 133 marks its blocks are made of. Found on 15 August by
following an unrelated question.

It was silent because every link degrades politely: the engine finds no directory and
shrugs, the curated .zshrc is never read, the integration script guards itself. Nothing
was broken; a feature simply was not there. And it looked fine in every dev run, because
the fallback exists in the checkout.

That is the class this reads: **a path the code names absolutely, that the image does not
provide**. Both halves are ordinary on their own - the code is right to have a system
location, and an image is allowed not to ship something - so only reading them together
shows the gap.

An asset root counts as provided when a `mkosi.build.d/*.chroot` step writes into it, or
when it exists under `mkosi.extra/usr/share/arlen/`. Everything else must be in
NOT_PROVIDED with a reason.

Only the FIRST component under `/usr/share/arlen/` is the unit, because that is the
granularity a step installs: `terminal/zdotdir` and `terminal/arlen-shell-integration.zsh`
are one decision. String literals inside `#[cfg(test)]` are skipped - a test naming
`/usr/share/arlen/a.jpeg` is exercising a path parser, not asking for a file.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ARGS = [a for a in sys.argv[1:] if a != "--unprovided"]
LIST_ONLY = "--unprovided" in sys.argv[1:]
ROOT = Path(ARGS[0]).resolve() if ARGS else Path(__file__).resolve().parents[2]

SCAN = ("apps", "daemons", "sdk", "ai")
STEPS = ROOT / "dev/mkosi/mkosi.build.d"
EXTRA = ROOT / "dev/mkosi/mkosi.extra/usr/share/arlen"

# Asset root to the reason the image does not provide it. Checked on 15 August.
NOT_PROVIDED: dict[str, str] = {
    "defaults": (
        "the system-defaults layer of the config system (`sdk/config`: "
        "/usr/share/arlen/defaults/{component}.toml under ~/.config/arlen). NOTHING in the "
        "tree ships a defaults file, so the documented two-layer config is one layer in "
        "practice and every component falls to its code defaults. That is coherent - there "
        "are no shipped defaults to ship - but it means the layer is untested, and the "
        "first component that relies on one will find out at runtime"
    ),
    "modules": (
        "the system-wide module directory (modulesd, Phase 7B). No first-party module "
        "exists to install, and both readers handle an absent directory, so an empty tree "
        "is the honest state rather than a missing install"
    ),
    "wallpaper": (
        "the distro default wallpaper manifest (`/usr/share/arlen/wallpaper/default.toml`, "
        "the wallpaper daemon's fallback when the user has set none). The image installs "
        "`/usr/share/arlen/wallpapers/` - the plural CATALOGUE the picker lists - and no "
        "manifest, so a fresh boot has no default wallpaper and the renderer paints "
        "nothing, showing the compositor's flat clear colour. That is deliberate in the "
        "daemon (a bad wallpaper config must never crash the background) and undecided in "
        "the image: there is no default image or manifest in the tree, so shipping one is "
        "an asset-and-design decision rather than a missing install line. NB the two names "
        "differ by one letter and mean different things, which is how a manual sweep "
        "matched `wallpaper` inside `wallpapers` and called this provided"
    ),
    "settings-schemas": (
        "read by the Settings app, which is not on the image either. It lands with "
        "whatever stages Settings; see `check-apps-on-image.py`"
    ),
    "version": (
        "read by the Settings About page, which is not on the image either - so nothing "
        "reads it today. It lands with whatever writes the OS version at image build, and "
        "that is the same decision as staging Settings"
    ),
}

LITERAL = re.compile(r'"/usr/share/arlen/([A-Za-z0-9._-]+)')


def roots_in_code() -> dict[str, str]:
    """Asset root to the first file that names it."""
    found: dict[str, str] = {}
    for area in SCAN:
        base = ROOT / area
        if not base.is_dir():
            continue
        for src in base.rglob("*.rs"):
            if "target" in src.parts or "node_modules" in src.parts:
                continue
            text = src.read_text(encoding="utf-8", errors="replace")
            # Test modules are conventionally last in a file; everything from the first
            # `#[cfg(test)]` on is a test naming paths, not a component reading them.
            cut = text.find("#[cfg(test)]")
            if cut != -1:
                text = text[:cut]
            for m in LITERAL.finditer(text):
                found.setdefault(m.group(1), str(src.relative_to(ROOT)))
    return found


def provided() -> set[str]:
    out: set[str] = set()
    if EXTRA.is_dir():
        out |= {p.name for p in EXTRA.iterdir()}
    if STEPS.is_dir():
        for step in STEPS.glob("*.chroot"):
            text = step.read_text(encoding="utf-8", errors="replace")
            for m in re.finditer(r"share/arlen/([A-Za-z0-9._-]+)", text):
                out.add(m.group(1))
    return out


def main() -> int:
    if LIST_ONLY:
        for name in sorted(NOT_PROVIDED):
            print(name)
        return 0
    if not (ROOT / "apps").is_dir() or not STEPS.is_dir():
        print("the layout moved and this check did not")
        return 1

    wanted = roots_in_code()
    if not wanted:
        print("no /usr/share/arlen path found in any component; that is not plausible")
        return 1
    have = provided()
    problems: list[str] = []

    for root, where in sorted(wanted.items()):
        if root in have:
            if root in NOT_PROVIDED:
                problems.append(
                    f"/usr/share/arlen/{root} IS provided by the image and is still listed "
                    f"in NOT_PROVIDED; drop the entry, a stale excuse says a gap is known "
                    f"when it is closed"
                )
            continue
        if root not in NOT_PROVIDED:
            problems.append(
                f"{where} reads /usr/share/arlen/{root} and no build step or extra puts it "
                f"there. On a real machine that lookup finds nothing - and if the code "
                f"degrades politely, as this kind usually does, the feature is simply "
                f"absent and nothing says so. Install it, or add it to NOT_PROVIDED with "
                f"the reason"
            )

    for name in sorted(NOT_PROVIDED):
        if name not in wanted:
            problems.append(
                f"{name} is listed in NOT_PROVIDED and no component reads it any more; "
                f"drop the entry"
            )

    if problems:
        print("\nthe code and the image disagree about runtime assets:\n")
        for p in problems:
            print(f"  - {p}")
        return 1

    print(
        f"{len(wanted)} runtime asset root(s) read by components: "
        f"{len(set(wanted) & have)} provided by the image, "
        f"{len(set(wanted) - have)} absent with a stated reason."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
