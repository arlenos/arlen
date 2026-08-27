# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that every profile the image ships is filed under an id something resolves to.

WHAT THIS IS FOR, in the words of the file that caused it. `compositor.toml` opens:
"RENAMED FROM `arlen-compositor.toml`, WHICH NOTHING EVER LOADED. The bus keys a
profile on the id it RESOLVES for the peer, and resolver rule (2) maps
`/usr/bin/arlen-compositor` to `compositor` - so a lookup went to `compositor.toml`
and found nothing, and everything below had been addressed to a principal that
never asked for it."

That is the failure this prevents. A profile is loaded by NAME, and the name has to
be the id the resolver produces for the running binary - not the binary's file
name, not the crate, not what the daemon calls itself. When they differ the lookup
finds nothing, and a peer with no profile declares no scope, which every reader
treats as "asked for nothing" rather than "could not be found". Both halves of that
compositor file were silently inert for weeks.

The producible set comes from `check-admitted-ids-exist`, imported rather than
re-derived: it already reads the resolver's strict arms and the build phases, it
already knows a convenience symlink mints no id, and a second copy of that rule is
one that goes stale on one side.

There is no acknowledgement list. A profile for a principal the image cannot
produce is not a decision somebody made, it is a file addressed to nobody.
"""

import importlib.util
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
PROFILES = ROOT / "dev/mkosi/mkosi.extra/var/lib/arlen/permissions/1000"
SIBLING = ROOT / "dev/scripts/check-admitted-ids-exist.py"


def producible_ids() -> dict[str, str]:
    """Every app id the image can resolve, borrowed from the sibling check."""
    spec = importlib.util.spec_from_file_location("admitted_ids", SIBLING)
    if spec is None or spec.loader is None:
        print(f"NOTHING WAS READ: cannot load {SIBLING}", file=sys.stderr)
        raise SystemExit(2)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.producible(ROOT)


def main() -> int:
    if not PROFILES.is_dir():
        print(f"NOTHING WAS READ: no profile directory at {PROFILES}", file=sys.stderr)
        return 2
    profiles = sorted(p.stem for p in PROFILES.glob("*.toml"))
    if not profiles:
        print(f"NOTHING WAS READ: no profiles in {PROFILES}", file=sys.stderr)
        return 2
    if not SIBLING.is_file():
        print(f"NOTHING WAS READ: no resolver reader at {SIBLING}", file=sys.stderr)
        return 2

    can_be = producible_ids()
    if not can_be:
        print(
            "NOTHING WAS READ: the image resolves no app id at all, so every "
            "profile would read as unaddressed",
            file=sys.stderr,
        )
        return 2

    findings = [
        f"{name}.toml is filed under `{name}`, and nothing the image stages "
        f"resolves to that id, so the file is never loaded and the principal it "
        f"grants reads as having asked for nothing"
        for name in profiles
        if name not in can_be
    ]
    if findings:
        print(f"{len(profiles)} shipped profile(s), {len(findings)} finding(s):\n", file=sys.stderr)
        for f in findings:
            print(f"  - {f}", file=sys.stderr)
        print(
            "\nRename the file to the id the resolver returns for that binary, "
            "rather than adding the id to the resolver: the id is what the peer "
            "IS, and the file name follows it.",
            file=sys.stderr,
        )
        return 1

    print(
        f"{len(profiles)} shipped profile(s), every one filed under an id the image "
        f"can resolve. A profile the lookup misses is not a weaker grant, it is no "
        f"grant at all, and it reads as an app that asked for nothing."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
