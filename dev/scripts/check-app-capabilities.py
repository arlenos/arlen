# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a Tauri app can actually reach the host commands it calls.

A Tauri app declares what its frontend may invoke in `src-tauri/capabilities/`.
With no file there, the frontend is denied EVERY host permission - and the denial
arrives as a rejected promise, which is exactly what `void invoke(...)` throws
away. The app runs, renders, and quietly does none of the things it asks the host
for.

TWO APPS SHIPPED LIKE THAT, and the symptom was invisible in English on a machine
with no other language configured. `apps/wine-manager` and `apps/mail` both had no
capabilities directory at all on 21 August, so:

  * `initArlenLocale()` was denied and both apps were English forever, with a
    complete German catalogue in the binary that nothing could reach. Note that
    `check-locale-adopted` is green for both: they DO call it. What it cannot see
    is the call being refused a layer down.
  * the window buttons did nothing, on windows built with `decorations: false` -
    a window a person could not move, minimise or close.

So this checks the layer that one cannot: not "does the app ask" but "is the app
allowed to ask". Every app with a `src-tauri` must carry a capabilities file.

WHY ONLY PRESENCE. The first cut of this also required the locale permission of
every app shipping a catalogue, and it flagged five more - the shell, the greeter,
settings, harness, store. Some of those are certainly fine: settings resolves the
language itself with `locale.set` because it owns the file the others read. The
rest I have not put in front of a running app, and a gate whose rule I cannot
defend for every app it fails is a gate people learn to override. The finding is
in `coder-reports.md` as a question instead, which is where an unverified claim
belongs.

What this does NOT cover:

  * Whether the permissions listed are the RIGHT ones, or whether an app lists
    more than it uses. Both are judgements about an app's design; this is about a
    file being absent entirely, which never is.
  * Which permissions an app needs. A gate that enumerated every command an app
    invokes against every permission it lists would be a second permission
    system, and a wrong one.
"""

import json
import sys
from pathlib import Path

ROOT = (
    Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
)

#: Keys are app directory names. A silence belongs here WITH its reason.
ACKNOWLEDGED: dict[str, str] = {}


def permissions_of(app: Path) -> tuple[list[str], list[str]]:
    """Every permission the app grants, and every file that would not parse."""
    granted: list[str] = []
    unreadable: list[str] = []
    for f in sorted((app / "src-tauri/capabilities").glob("*.json")):
        try:
            doc = json.loads(f.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as e:
            unreadable.append(f"{f.name}: {e}")
            continue
        for p in doc.get("permissions", []):
            # A permission can be a string or an object with an identifier.
            granted.append(p if isinstance(p, str) else str(p.get("identifier", "")))
    return granted, unreadable


def main() -> int:
    apps = ROOT / "apps"
    if not apps.is_dir():
        print(f"NOTHING WAS READ: no apps under {apps}", file=sys.stderr)
        return 2

    findings: list[str] = []
    checked = 0
    for app in sorted(p for p in apps.iterdir() if (p / "src-tauri").is_dir()):
        if app.name in ACKNOWLEDGED:
            continue
        checked += 1
        granted, unreadable = permissions_of(app)
        for bad in unreadable:
            findings.append(f"apps/{app.name}: a capabilities file will not parse - {bad}")
        if not granted and not unreadable:
            findings.append(
                f"apps/{app.name}: has no capabilities file, so its frontend is denied every "
                f"host permission and every `invoke` it makes is refused"
            )
            continue

    if not checked:
        print("NOTHING WAS READ: no app carries a src-tauri", file=sys.stderr)
        return 2

    print(
        f"{checked} app(s) checked for a capabilities file, "
        f"{len(ACKNOWLEDGED)} acknowledged with a reason. Presence only: which "
        f"permissions an app needs is a design question, not a checkable one."
    )
    if findings:
        print("\napps that cannot reach the host they call:\n")
        for f in findings:
            print(f"  - {f}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
