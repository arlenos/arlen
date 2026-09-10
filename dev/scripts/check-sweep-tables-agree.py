#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that the render sweep and the axe sweep look at the same surfaces.

Two tables name what gets looked at: `sweep-render-all.sh` (layout, focus rings,
overlap) and `sweep-axe.sh` (the accessibility audit). Both make the same claim
about themselves - the list IS the coverage, and a surface not in it is invisible
rather than clean - and they are maintained by hand, one row at a time, usually
when a new fixture lands.

MEASURED, on 11 September: the render table had 29 rows the axe table did not,
and every one of them was a REFUSAL state - a save that failed, a login refused,
an attachment that would not write. Those are the surfaces most likely to carry
an a11y defect (an error nobody announces, a dialog with no name), they were the
most expensive ones to build, and the accessibility audit had never seen one of
them. Nothing said so: both sweeps were green.

THE RULE: every (route, selector, host) the render table names, the axe table
names too. Queries are ignored on purpose - a fixture row pins `?locale=de` in
the axe table and often does not in the render table, and that difference is
deliberate.

WHAT IT DOES NOT SAY: that either table is complete. A route in neither is
invisible to both, and only a diff against the routes on disk can see that -
which is what the header of `sweep-render-all.sh` describes doing by hand.

Run: dev/scripts/check-sweep-tables-agree.py [root]
"""

import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
RENDER = ROOT / "dev/screenshot/sweep-render-all.sh"
AXE = ROOT / "dev/screenshot/sweep-axe.sh"

#: App to the rows the axe table may leave out, with the reason. Empty: the bar
#: for adding one is an argument that the surface cannot be audited at all, not
#: that nobody has got to it.
CARRIED: dict[str, str] = {}


def surface(spec: str) -> tuple[str, str, str]:
    """A row reduced to what both tables mean by it: route, click selector, host.

    The query string is dropped: the axe table pins a locale on its fixture rows
    and the render table often does not, which is a difference in how each sweep
    runs rather than in what it looks at.
    """
    host = spec.split("@@")[1] if "@@" in spec else ""
    head = spec.split("@@")[0]
    route = head.split("::")[0].split("?")[0] or "/"
    sel = head.split("::")[1] if "::" in head else ""
    return (route, sel, host)


def table(path: Path) -> dict[str, set[tuple[str, str, str]]]:
    text = path.read_text(encoding="utf-8", errors="replace")
    if "SURFACES=(" not in text:
        return {}
    block = text.split("SURFACES=(", 1)[1].split("\n)", 1)[0]
    out: dict[str, set[tuple[str, str, str]]] = {}
    for line in block.splitlines():
        line = line.strip()
        if not line.startswith('"') or not line.endswith('"'):
            continue
        app, _, routes = line.strip('"').partition(" ")
        out[app] = {surface(r) for r in routes.split("|") if r}
    return out


def main() -> int:
    for path in (RENDER, AXE):
        if not path.is_file():
            print(f"check-sweep-tables-agree: no {path}; the scan is pointed wrong")
            return 1
    render, axe = table(RENDER), table(AXE)
    if not render or not axe:
        print("check-sweep-tables-agree: a table parsed to nothing, so the scan is pointed wrong")
        return 1

    bad: list[str] = []
    carried = 0
    for app in sorted(render):
        if app not in axe:
            bad.append(f"  - {app}: the render sweep looks at it and the axe sweep names no row for it at all.")
            continue
        for route, sel, host in sorted(render[app] - axe[app]):
            spec = route + (f"::{sel}" if sel else "") + (f"@@{host}" if host else "")
            if CARRIED.get(f"{app} {spec}"):
                carried += 1
                continue
            bad.append(
                f"  - {app} {spec}: the render sweep looks at this surface and the\n"
                f"    accessibility audit does not. Add the row to sweep-axe.sh, pinning\n"
                f"    the locale the way its neighbours do."
            )

    total = sum(len(v) for v in render.values())
    print(
        f"{total} surface(s) in the render table across {len(render)} app(s); the axe table"
        f" names every one, {carried} carried with a reason."
    )
    if bad:
        print("\nsurfaces the accessibility audit never sees:\n")
        print("\n".join(bad))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
