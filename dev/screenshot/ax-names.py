# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Find focusable elements a screen reader cannot name, by asking the engine.

The tempting version of this check is a grep for `<button>` without an `aria-label`.
That was tried and thrown away: it cannot tell an interpolation that renders text from
one that does not, so it reported every translated button in the tree as unnamed. A
checker that cries wolf teaches people to ignore it.

This asks WebKit instead. WebDriver's `computedlabel` and `computedrole` endpoints
return the accessible name and role the accessibility tree actually exposes - after
`aria-label`, `aria-labelledby`, `title`, the element's own text, and everything else
the name-computation algorithm folds in. If this says a control has no name, a screen
reader announces "button" and nothing more.

Usage:
    dev/screenshot/webkit-headless.sh 4494 &
    # a dev server for the app under test, then:
    python3 dev/screenshot/ax-names.py http://localhost:5181 /appearance /display ...

Exits 1 if any focusable element has an empty name.
"""

import json
import sys
import time
import urllib.error
import urllib.request

DRIVER = "http://127.0.0.1:4494"

# Things a user can reach with Tab and act on, plus the two header roles. A named
# element is one whose accessible name is non-empty; that is the whole test.
#
# The header roles are in the list because WebKit - which is the engine our apps
# actually run in - computes no name from a `columnheader`'s contents, only from
# `aria-label`/`aria-labelledby`. Probed directly: `<button role="columnheader">Name`
# reports an empty name while the same button without the role reports "Name". So a
# sortable column header written the obvious way labels its column for a sighted user
# and for nobody else. `row` and `gridcell` behave the same way but are containers a
# reader may fall back to reading the contents of, so they are left out rather than
# reported on a guess.
FOCUSABLE = (
    "button, a[href], input, select, textarea, [tabindex]:not([tabindex='-1']), "
    "[role='columnheader'], [role='rowheader']"
)


def req(method, path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    r = urllib.request.Request(
        DRIVER + path, data, {"Content-Type": "application/json"}, method=method
    )
    try:
        return json.loads(urllib.request.urlopen(r, timeout=60).read())
    except urllib.error.HTTPError as e:
        return {"error": e.code, "body": e.read().decode()[:300]}


def main(argv):
    if len(argv) < 3:
        print(__doc__)
        return 2
    base, routes = argv[1], argv[2:]

    session = req(
        "POST", "/session", {"capabilities": {"alwaysMatch": {"browserName": "MiniBrowser"}}}
    )
    if "value" not in session or "sessionId" not in session.get("value", {}):
        print(f"cannot start a WebKit session: {session}", file=sys.stderr)
        print("is dev/screenshot/webkit-headless.sh running on 4494?", file=sys.stderr)
        return 2
    sid = session["value"]["sessionId"]

    unnamed, checked = [], 0
    try:
        for route in routes:
            req("POST", f"/session/{sid}/url", {"url": base + route})
            # The apps render after their stores settle; a shorter wait photographs a
            # half-built page and reports controls that do not exist yet.
            time.sleep(5)
            found = req(
                "POST", f"/session/{sid}/elements", {"using": "css selector", "value": FOCUSABLE}
            ).get("value", [])
            for el in found:
                eid = list(el.values())[0]
                # Skip what the user cannot reach: an invisible control has no name to
                # announce and is not a finding.
                shown = req("GET", f"/session/{sid}/element/{eid}/displayed").get("value")
                if shown is not True:
                    continue
                checked += 1
                label = (req("GET", f"/session/{sid}/element/{eid}/computedlabel").get("value") or "").strip()
                if label:
                    continue
                role = req("GET", f"/session/{sid}/element/{eid}/computedrole").get("value") or "?"
                text = (req("GET", f"/session/{sid}/element/{eid}/text").get("value") or "").strip()
                outer = req(
                    "POST",
                    f"/session/{sid}/execute/sync",
                    {"script": "return arguments[0].outerHTML.slice(0, 120)", "args": [el]},
                ).get("value", "")
                unnamed.append(f"{route}  role={role}  text={text!r}\n      {outer}")
    finally:
        req("DELETE", f"/session/{sid}")

    print(f"{checked} visible focusable element(s) across {len(routes)} route(s)")
    if unnamed:
        print(f"\n{len(unnamed)} with no accessible name:\n")
        for u in unnamed:
            print("  - " + u)
        return 1
    print("every one of them has an accessible name")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
