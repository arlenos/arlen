#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that every route on disk is named by the render sweep.

`sweep-render-all.sh` says of itself that the list IS the coverage, and a surface
missing from it is invisible rather than clean. Its sibling gate
(`check-sweep-tables-agree.py`) makes the axe table agree with it, and its own
header says what neither of them can see: a route that is in NEITHER table is
invisible to both, and only a diff against what is on disk finds it.

That diff was a thing somebody did by hand, occasionally, which is the same shape
as the coverage gaps it is meant to catch. This is the diff, run every time.

WHAT COUNTS AS A ROUTE: a `+page.svelte` under an app's `src/routes`. Group
segments (`(app)`) are not part of the URL and are dropped; a dynamic segment
(`[id]`) matches whatever concrete value the table pins, because the table has to
name a real one to render it.

WHAT IT DOES NOT SAY: that a named route is looked at WELL. A row may name a
route and never click the thing on it that matters. Selectors and hosts are the
other tables' business.

Run: dev/scripts/check-routes-are-swept.py [root]
"""

import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
RENDER = ROOT / "dev/screenshot/sweep-render-all.sh"

#: Routes the render table may leave out, keyed `<app>` for a whole app or
#: `<app>:<route>` for one route, with the reason.
#:
#: The bar for adding one is an argument that the surface is not this lane's to
#: sweep, not that nobody has got to it yet.
CARRIED: dict[str, str] = {
    "harness": "arlen-ui's live work; a shared sweep that goes red on another"
    " lane's surface is a sweep somebody turns off",
    "store": "arlen-ui's live work, same reason",
    "settings:/ai/models": "arlen-ui owns this page and the one under it",
    "settings:/ai/models/get": "arlen-ui owns this page",
}


def named_routes(text: str) -> dict[str, set[str]]:
    """App to the routes its rows name, with query, selector and host stripped."""
    out: dict[str, set[str]] = {}
    for m in re.finditer(r'^\s*"([a-z0-9-]+) (.+)"\s*$', text, re.M):
        app, specs = m.group(1), m.group(2)
        for spec in specs.split("|"):
            head = spec.split("@@")[0].split("::")[0].split("?")[0]
            out.setdefault(app, set()).add(head or "/")
    return out


def routes_on_disk(app_src: Path) -> set[str]:
    """Every `+page.svelte` under `src/routes`, as the URL it answers."""
    base = app_src / "routes"
    if not base.is_dir():
        return set()
    out = set()
    for page in base.rglob("+page.svelte"):
        parts = [
            p
            for p in page.relative_to(base).parent.parts
            if not (p.startswith("(") and p.endswith(")"))
        ]
        out.add("/" + "/".join(parts) if parts else "/")
    return out


def is_named(route: str, named: set[str]) -> bool:
    """True when some named route matches, treating `[id]` as any one segment."""
    want = [] if route == "/" else route.strip("/").split("/")
    for candidate in named:
        have = [] if candidate == "/" else candidate.strip("/").split("/")
        if len(have) != len(want):
            continue
        if all(
            w == h or (w.startswith("[") and w.endswith("]")) for w, h in zip(want, have)
        ):
            return True
    return False


def main() -> int:
    if not RENDER.is_file():
        print(f"NOTHING WAS READ: no render table at {RENDER}", file=sys.stderr)
        return 2
    apps_dir = ROOT / "apps"
    apps = (
        sorted(p for p in apps_dir.iterdir() if (p / "src" / "routes").is_dir())
        if apps_dir.is_dir()
        else []
    )
    if not apps:
        print(f"NOTHING WAS READ: no app with routes under {apps_dir}", file=sys.stderr)
        return 2

    named = named_routes(RENDER.read_text(encoding="utf-8"))
    findings: list[str] = []
    checked = 0
    for app in apps:
        if app.name in CARRIED:
            continue
        for route in sorted(routes_on_disk(app / "src")):
            if f"{app.name}:{route}" in CARRIED:
                continue
            checked += 1
            if not is_named(route, named.get(app.name, set())):
                findings.append(
                    f"  - {app.name} {route}: on disk and named by no row in"
                    f" sweep-render-all.sh, so nothing has ever looked at it."
                )

    if findings:
        print("A route ships that no sweep looks at:", file=sys.stderr)
        for f in findings:
            print(f, file=sys.stderr)
        print(
            "\nAdd it to the app's row in dev/screenshot/sweep-render-all.sh (and to"
            " sweep-axe.sh, which its sibling gate checks), or to CARRIED with the"
            " reason it is not this lane's.",
            file=sys.stderr,
        )
        return 1

    print(
        f"{checked} route(s) across {len(apps) - len([a for a in apps if a.name in CARRIED])}"
        f" app(s) are on disk, and every one of them is named by the render sweep."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
