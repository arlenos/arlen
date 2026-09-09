# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that another state cannot take a control's focus indication away.

Two shapes, both found on the night of 6 September, in two different apps and an
hour apart:

    .field:focus-within { border-color: ... }   /* the only focus affordance */
    .field.error        { border-color: ... }   /* later, same specificity */

    .swatch.active { outline: 2px solid ... }   /* selection took the outline */
    /* ...and nothing draws focus, so the UA ring is suppressed too */

Both are one thing: focus and another state competing for a single channel, and
the other state winning. The greeter's password field showed no focus at all
while it was red - exactly when somebody is about to retype it - and the
screenshot editor's selected swatch could not show focus because selection had
already claimed the outline.

WHY THIS IS NOT THE RENDER PROBE'S JOB. `dev/screenshot/no-focus-ring.js` finds
this by focusing a real control on a real surface, which is stronger evidence and
much narrower coverage: it only ever sees a surface somebody stood a server up
for and drove into the right state. The greeter's case needed the field to be in
its ERROR state to show at all. This reads the CSS instead, so it covers every
component in the tree including the states nobody has photographed yet, and it
runs in a second.

What it looks for:

  1. a `:focus` / `:focus-visible` / `:focus-within` rule whose every declared
     property is also declared by a LATER rule on the same base selector carrying
     a state class - `.error`, `.active`, `.selected`, `.invalid`, `.checked`,
     `.open`, `.current`, or an `aria-` attribute
  2. a state rule that claims `outline` in a component with no focus rule at all,
     which suppresses the browser's own ring and puts nothing back

What it does NOT cover:

  * specificity arithmetic. It assumes the state rule wins, which is true when
    they are equal and the state comes later - the greeter's case - and
    pessimistic otherwise. A false positive here is one comment away from being
    a carried entry with a reason
  * a focus style that lives in a global stylesheet or a parent component
  * whether the surviving affordance is VISIBLE ENOUGH. Contrast is axe's
    question and a person's
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
SKIP = ("node_modules", "/.svelte-kit/", "/harness/", "/store/")

RULE = re.compile(r"([^{}]+)\{([^{}]*)\}", re.S)
PROP = re.compile(r"(?:^|;)\s*([a-z-]+)\s*:")
STATE = re.compile(r"\.(active|selected|error|invalid|on|current|checked|open|sel)\b|\[aria-")

KNOWN: dict[str, tuple[int, str]] = {}


def rules(css: str) -> list[tuple[str, str]]:
    out = []
    for m in RULE.finditer(css):
        selector = m.group(1).strip().split("\n")[-1].strip()
        out.append((selector, m.group(2)))
    return out


def props(body: str) -> set[str]:
    return {m.group(1) for m in PROP.finditer(body)} - {""}


def findings_in(text: str) -> list[str]:
    start = text.find("<style")
    if start < 0:
        return []
    css = text[start:]
    rs = rules(css)
    focus = [(s, b) for s, b in rs if ":focus" in s]
    out: list[str] = []

    for fsel, fbody in focus:
        changed = props(fbody)
        base = fsel.split(":focus")[0].strip()
        if not changed or not base:
            continue
        for ssel, sbody in rs:
            if ssel == fsel or ":focus" in ssel:
                continue
            if not ssel.startswith(base) or ssel == base:
                continue
            if not STATE.search(ssel):
                continue
            if changed <= props(sbody):
                out.append(
                    f"`{fsel}` shows focus with {', '.join(sorted(changed))},"
                    f" and `{ssel}` sets all of it"
                )

    if not focus:
        for ssel, sbody in rs:
            if ":focus" in ssel or not STATE.search(ssel):
                continue
            m = re.search(r"(?:^|;)\s*outline\s*:\s*([^;]*)", sbody)
            if not m or "none" in m.group(1):
                continue
            out.append(
                f"`{ssel}` claims the outline for a state, and nothing in this"
                " component draws focus - so the browser's own ring is suppressed"
                " with nothing put back"
            )
    return out


def sources() -> list[Path]:
    out: list[Path] = []
    for base in (ROOT / "apps", ROOT / "sdk" / "ui-kit" / "src"):
        for p in sorted(base.rglob("*.svelte")):
            if any(s in "/" + str(p.relative_to(ROOT)) for s in SKIP):
                continue
            out.append(p)
    return out


def main() -> int:
    files = sources()
    if not files:
        print("check-focus-survives-state: no frontend sources found, so the scan is pointed wrong")
        return 1
    bad: list[str] = []
    carried = 0
    for p in files:
        rel = str(p.relative_to(ROOT))
        hits = findings_in(p.read_text(encoding="utf-8", errors="replace"))
        allowed = KNOWN.get(rel, (0, ""))[0]
        if len(hits) > allowed:
            bad.extend(f"  - {rel}: {h}" for h in hits)
        else:
            carried += len(hits)
    print(
        f"{len(files)} component(s) checked for a focus indication another state can"
        f" take away. {carried} carried in {len(KNOWN)} known file(s)."
        " Specificity is assumed rather than computed, so a state rule is taken to"
        " win; a focus style in a global sheet or a parent is not visible here."
    )
    if bad:
        print("\nfocus indications a state can take away:\n")
        print("\n".join(bad))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
