#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a shell surface an app is granted is one it actually calls.

`check-plugin-grants` asks the question one way round - an app that CALLS a
surface must be granted it, or the call is refused at runtime. This asks the
other: a grant nothing exercises is authority sitting in a capability file for
whoever gets into that app, and it is invisible because nothing fails.

The one this was written for: the file manager held
`arlen-shell:allow-shortcuts-clear` beside its `register` grant and never called
`shortcuts.clear()`. Nothing takes a shortcut list down while its window stays
open, and a window that goes is reclaimed by the shell on observed absence - so
the capability had no caller and could not acquire one without somebody writing
the call. It came out with its publish scope the day this check was written.

MATCHED AS `object.method(`, not as a bare method name. The API is a set of small
objects (`ambient.set`, `presence.clear`, `shortcuts.register`), and matching the
method alone would count any `set(` in the app - which is every store in the
tree, so the check would pass on everything and mean nothing. A destructured call
(`const { set } = ambient`) would defeat the pair match; there is none in the
tree today, and `test-check-grants-exercised.mjs` holds the case so a future one
is a control failure rather than a silent pass.

NOT A SECURITY BOUNDARY BY ITSELF. The capability file is compiled into the
binary, so this is about what the app was BUILT able to do, not what it may do at
runtime - the profile's publish scope is the other half and
`check-publish-grants` covers it. What this catches is the grant that outlived
its caller.

Run: dev/scripts/check-grants-exercised.py [root]
"""

import re
import sys
from pathlib import Path

#: Apps holding a grant they never call, with why. EMPTY: the one this was
#: written for came out the same day, and an entry here is authority somebody
#: decided to leave in a binary.
CARRIED: dict[str, str] = {}

GRANT = re.compile(r'"arlen-shell:allow-([a-z-]+)"')
OBJECT = re.compile(r"export const (\w+) = \{")
METHOD = re.compile(r"async (\w+)\s*[(<]")
COMMAND = re.compile(r"\$\{PLUGIN\}\|(\w+)")
FRONTEND = (".ts", ".svelte")


def wrapper_calls(api: str) -> dict[str, set[tuple[str, str]]]:
    """`command -> {(object, method)}` read out of the TS wrapper's own bodies."""
    out: dict[str, set[tuple[str, str]]] = {}
    obj = method = None
    for line in api.splitlines():
        m = OBJECT.match(line)
        if m:
            obj = m.group(1)
            continue
        m = METHOD.search(line)
        if m:
            method = m.group(1)
            continue
        m = COMMAND.search(line)
        if m and obj and method:
            out.setdefault(m.group(1), set()).add((obj, method))
    return out


def frontend_text(app_src: Path) -> str:
    parts = []
    for path in app_src.rglob("*"):
        if path.suffix not in FRONTEND or "node_modules" in str(path):
            continue
        try:
            parts.append(path.read_text(errors="replace"))
        except OSError:
            continue
    return "\n".join(parts)


def main() -> int:
    root = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
    api = root / "sdk/tauri-plugin-shell/index.ts"
    if not api.is_file():
        print("check-grants-exercised: no shell plugin API, so the scan is pointed wrong")
        return 1
    calls = wrapper_calls(api.read_text(errors="replace"))

    checked = 0
    problems: list[str] = []
    for cap in sorted((root / "apps").glob("*/src-tauri/capabilities/default.json")):
        app = cap.parts[-4]
        src = cap.parents[2] / "src"
        if not src.is_dir():
            continue
        granted = GRANT.findall(cap.read_text(errors="replace"))
        # Only the grants that name a surface this API exposes; `theme-get` and
        # the like are not shell surfaces with a wrapper to look for.
        pairs = {g: calls.get(g.replace("-", "_"), set()) for g in granted}
        pairs = {g: p for g, p in pairs.items() if p}
        if not pairs:
            continue
        checked += 1
        text = frontend_text(src)
        dark = sorted(
            g for g, p in pairs.items() if not any(f"{o}.{m}(" in text for o, m in p)
        )
        if dark and app not in CARRIED:
            problems.append(f"  - {app}: granted and never called: {', '.join(dark)}")

    if problems:
        print("Shell surfaces an app may reach and never does:\n")
        print("\n".join(problems))
        print(
            "\n  A grant nothing exercises is authority in a binary with no caller."
            "\n  Call it, or give it back - it comes back in the change that needs it."
        )
        return 1

    print(
        f"check-grants-exercised: {checked} app(s) with shell grants; "
        "each calls every surface it holds"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
