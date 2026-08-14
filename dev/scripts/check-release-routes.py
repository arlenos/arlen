#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""A dev harness route must not reach a user's machine.

Every app keeps `_`-prefixed routes so the states nobody can reach on purpose can
still be rendered and photographed - a sidebar's refusal sentence, a toggle that
could not be saved. They are how anything gets looked at, so they stay in the dev
and verify builds. On a user's machine they are test surfaces wired to real
commands with mock data: doors nobody designed for a person to open.

They used to ship. A production build of the greeter emitted the compiled
`_a11ytest` page, its stylesheet, and a route-manifest entry naming it, so an app
pointed at that path rendered the harness. `dev/build/release-routes.js` fixes
that by pointing `kit.files.routes` at a symlink farm without them when
`ARLEN_RELEASE` is set.

THIS CHECKS BOTH HALVES, because each fails differently and silently:

  1. STATIC - every app that has harness routes wires the exclusion. An app added
     later, or a `svelte.config.js` rewritten without it, ships them again and
     nothing else notices.

  2. BUILT - the emitted files contain no harness route. This is the one that
     cannot be fooled by config that looks right, because it reads what was
     actually produced.

THE BUILT HALF ONLY RUNS WHEN ASKED (`--built`), and that is not laziness. An
`apps/<x>/build` directory on a working tree is almost always a DEV build, which
contains every harness route and should - so a check that inspected whatever it
found would go red on a healthy tree, which is how a check gets switched off
rather than read. Nothing in the output distinguishes the two builds, so the
caller has to say. The release pipeline runs `--built` right after building;
everything else gets the static half.
"""

import re
import sys
from pathlib import Path

ARGS = [a for a in sys.argv[1:] if a != "--built"]
BUILT = "--built" in sys.argv
OWN_TREE = not ARGS
REPO = Path(__file__).resolve().parents[2] if OWN_TREE else Path(ARGS[0]).resolve()

APPS = "apps"

# Apps whose build is not ours to configure. The harness is arlen-ui's live work;
# saying so here is better than the check quietly skipping it.
NOT_OURS = {"harness": "arlen-ui's live work"}

WIRED = re.compile(r"routesDir\s*\(")


def dev_routes(app: Path) -> list[str]:
    routes = app / "src" / "routes"
    if not routes.is_dir():
        return []
    return sorted(d.name for d in routes.iterdir() if d.is_dir() and d.name.startswith("_"))


def built_leaks(app: Path) -> list[str]:
    """Harness route names that appear anywhere in this app's build output."""
    build = app / "build"
    if not BUILT or not build.is_dir():
        return []
    names = dev_routes(app)
    if not names:
        return []
    found = set()
    for f in build.rglob("*"):
        if not f.is_file():
            continue
        try:
            text = f.read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        for n in names:
            if n in text:
                found.add(n)
    return sorted(found)


def main() -> int:
    base = REPO / APPS
    if not base.is_dir():
        print(f"NOTHING WAS READ: no {APPS}/ under {REPO}", file=sys.stderr)
        return 2

    apps = sorted(d for d in base.iterdir() if (d / "svelte.config.js").is_file())
    if not apps:
        print(f"NOTHING WAS READ: no app carries a svelte.config.js under {base}", file=sys.stderr)
        return 2

    problems: list[str] = []
    carried: list[str] = []
    wired = 0
    built = 0

    for app in apps:
        routes = dev_routes(app)
        if not routes:
            continue
        if app.name in NOT_OURS:
            carried.append(f"{app.name}: {len(routes)} harness route(s), {NOT_OURS[app.name]}")
            continue

        config = (app / "svelte.config.js").read_text(encoding="utf-8")
        if not WIRED.search(config):
            problems.append(
                f"{app.name}: has {', '.join(routes)} and its svelte.config.js does not "
                f"call `routesDir`.\n"
                f"    Without it a release build ships those pages, their chunks and "
                f"their CSS, and the route manifest names them - so the app renders a "
                f"test surface for anyone who asks for the path. Import `routesDir` "
                f"from dev/build/release-routes.js and set "
                f"`kit.files.routes`."
            )
        else:
            wired += 1

        leaked = built_leaks(app)
        if leaked:
            built += 1
            problems.append(
                f"{app.name}: a build in {app.name}/build still names "
                f"{', '.join(leaked)}.\n"
                f"    You asked for --built, so this is being read as a release "
                f"artefact: the exclusion is not working and these pages are in what "
                f"ships. If it was actually a dev build, the answer is correct and "
                f"the question was wrong - rebuild with ARLEN_RELEASE=1 first."
            )

    if carried:
        print("carried, with a reason (see NOT_OURS):")
        for line in carried:
            print(f"  {line}")
        print()

    if problems:
        print("a dev harness route that could reach a user:")
        for p in problems:
            print(f"  {p}")
        return 1

    print(
        f"OK: {wired} app(s) with harness routes exclude them from a release build"
        + (
            "; and no release build names one"
            if BUILT
            else "; pass --built after a release build to check the emitted files too"
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
