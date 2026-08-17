# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a profile granting the whole network says why it is not scoped.

The permission grammar has three network settings and only two behave. A host
list (`allowed_domains`) becomes `NetworkPolicy::FilteredHosts`, which
`arlen-run` REFUSES with exit 67 unless an egress enforcer is installed - and
that enforcer is the metal-gated piece. Nothing raises a `NetworkAccess` consent
for a host an app did not declare either. So the choice today is no network at
all or all of it, and 707 profiles took all of it.

That is the right call while the middle is unenforceable: narrowing would leave
those apps with no network and no way to ask, which is the silent-break class.
What was wrong was recording it in several shapes, or not at all, so the set that
needs revisiting when the enforcer lands would have been archaeology.

Hence one form of words, `NETWORK-SCOPE-PENDING`, in every profile that grants
`allow_all` without a host list. The point is the grep: when the enforcer and the
consent producer land, that marker IS the work list.

A profile that scopes its network with `allowed_domains` needs no marker - it is
already saying the narrow thing - and neither does one with no network at all.
"""

import sys
import tomllib
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
PROFILES = ROOT / "sdk/permissions/profiles"

MARKER = "NETWORK-SCOPE-PENDING"


def main() -> int:
    if not PROFILES.is_dir():
        print(f"NOTHING WAS READ: no profiles at {PROFILES}", file=sys.stderr)
        return 2

    missing: list[str] = []
    stale: list[str] = []
    wide = 0
    read = 0

    for path in sorted(PROFILES.glob("*.toml")):
        text = path.read_text(encoding="utf-8")
        try:
            doc = tomllib.loads(text)
        except tomllib.TOMLDecodeError as e:
            print(f"{path.name}: does not parse ({e})", file=sys.stderr)
            return 1
        read += 1
        net = doc.get("network", {})
        unscoped = bool(net.get("allow_all")) and not net.get("allowed_domains")
        marked = MARKER in text
        if unscoped:
            wide += 1
            if not marked:
                missing.append(path.name)
        elif marked:
            # The marker outliving the grant it explains is its own small lie, and
            # the whole value here is that the grep names exactly the work.
            stale.append(path.name)

    if not read:
        print("NOTHING WAS READ: no profile parsed", file=sys.stderr)
        return 2

    print(
        f"{read} profile(s) checked; {wide} grant the whole network and carry the "
        f"{MARKER} note. The note is the work list for the day a scoped host list "
        f"becomes enforceable."
    )
    if missing:
        print(f"\nwide network grants that do not say why they are not scoped:\n", file=sys.stderr)
        for m in missing:
            print(f"  - {m}", file=sys.stderr)
        print(
            f"\nAdd the {MARKER} note above the [network] table. Narrowing instead is "
            "not the fix today: `arlen-run` refuses a host list without an egress "
            "enforcer, so the app would get no network and no way to ask.",
            file=sys.stderr,
        )
    if stale:
        print(f"\n{MARKER} on a profile that is no longer network-wide:\n", file=sys.stderr)
        for s in stale:
            print(f"  - {s}", file=sys.stderr)
        print("\nDrop the note; it explains a grant that is not there.", file=sys.stderr)
    return 1 if (missing or stale) else 0


if __name__ == "__main__":
    raise SystemExit(main())
