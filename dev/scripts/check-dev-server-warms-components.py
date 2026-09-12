#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that every app's dev server pre-transforms its components.

Vite serves each `<style>` block as its own module. When the browser asks for one
before the plugin has transformed the component, there is no compiled CSS to hand
back and the RAW `.svelte` source is injected as that component's stylesheet: the
browser recovers at the first thing that parses as a rule, so the sheet arrives
unscoped and missing whatever sat before that point.

Measured on 11 September: 42 of the shell's 63 component stylesheets, cold server
and warm, while `files` and `settings` were clean. It only shows on a page that
pulls in enough components at once for the style requests to outrun the
transforms - so the app that most needed looking at was the one being looked at
wrong. Everything the harness renders is a dev server, so that was what every
screenshot and every axe run of that surface had been measuring. `vite build` is
unaffected.

`server.warmup.clientFiles` fixes it by pre-transforming at server start.
`render-wide.py` refuses a page that still has a raw one, which catches it at
render time; this catches it at commit time, so a new app cannot ship without the
line and wait to be found by a sweep.

WHAT IT DOES NOT CHECK: that the warmup finishes before the first render. Nothing
static can. That is the renderer's refusal, and the two together are the pair.

Run: dev/scripts/check-dev-server-warms-components.py [root]
"""

import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

#: The call that does it, matched on the parts that matter: the key, and a glob
#: that reaches this app's own `.svelte` files.
WARMUP = re.compile(r"warmup\s*:\s*\{[^}]*clientFiles\s*:\s*\[[^\]]*\*\*/\*\.svelte")

#: Apps this gate does not hold, with the reason. NOT "does not need it" - both
#: of these need it exactly as much as the others, and the finding is recorded in
#: `coder-reports.md` with the line to add. They are arlen-ui's live work and this
#: lane does not write in `apps/harness/**` or `apps/store/**`, so carrying them
#: is the boundary, not a judgement about the defect. Drop the entry the moment
#: the line lands.
CARRIED: dict[str, str] = {
    "harness": "arlen-ui's lane; the one-line fix is theirs to add",
    "store": "arlen-ui's lane; the one-line fix is theirs to add",
}


def main() -> int:
    apps_dir = ROOT / "apps"
    apps = (
        sorted(p for p in apps_dir.iterdir() if (p / "src").is_dir())
        if apps_dir.is_dir()
        else []
    )
    if not apps:
        print(f"NOTHING WAS READ: no app under {apps_dir}", file=sys.stderr)
        return 2

    findings: list[str] = []
    checked = 0
    for app in apps:
        config = next(
            (app / name for name in ("vite.config.js", "vite.config.ts") if (app / name).is_file()),
            None,
        )
        if config is None:
            continue
        if app.name in CARRIED:
            continue
        checked += 1
        if not WARMUP.search(config.read_text(encoding="utf-8")):
            findings.append(
                f"  - {config.relative_to(ROOT)}: no"
                f" `server.warmup.clientFiles` reaching this app's `.svelte`"
                f" files, so its dev server can serve a component's own source"
                f" as that component's stylesheet."
            )

    if findings:
        print(
            "An app's dev server does not warm its components:", file=sys.stderr
        )
        for f in findings:
            print(f, file=sys.stderr)
        print(
            '\nAdd `warmup: { clientFiles: ["./src/**/*.svelte"] }` to the'
            " `server` block, or add the app to CARRIED with the reason it needs"
            " no dev server.",
            file=sys.stderr,
        )
        return 1

    print(
        f"{checked} app dev server(s) pre-transform their components, so none of"
        " them can serve a component's source as its CSS."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
