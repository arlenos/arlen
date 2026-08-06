#!/usr/bin/env python3
"""Test Layer 1b WebDriver client: load a URL in an already-running
WebKitWebDriver, optionally run an injection script, and save a screenshot.

Assumes a WebKitWebDriver is listening on --port (shoot.sh starts one under
Xvfb). Kept dependency-free (stdlib only) so the harness needs no venv.
"""
import argparse
import base64
import json
import sys
import time
import urllib.request


def rq(base, method, path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(
        base + path, data=data, method=method,
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.load(r)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--url", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--inject", default=None, help="path to a JS file run after load")
    ap.add_argument("--port", type=int, default=4477)
    ap.add_argument("--width", type=int, default=1280)
    ap.add_argument("--height", type=int, default=800)
    ap.add_argument("--settle", type=float, default=1.5, help="seconds to wait after load")
    # An injection that drives the page - stubbing an IPC shim and re-navigating so a
    # data-bearing view re-mounts through it - needs longer than a repaint. Without
    # this the shot lands mid-navigation and reads as "the page is broken".
    ap.add_argument("--after", type=float, default=0.8,
                    help="seconds to wait after the injection, before the screenshot")
    # Anything behind a click was outside every check built on this: the id scan
    # reported a dialog's routes clean while never opening the dialog. One click
    # before the inject is the cheapest thing that reaches that half of the UI.
    ap.add_argument("--open", dest="open_selector", default=None,
                    help="CSS selector to click after load, before the injection")
    args = ap.parse_args()

    base = f"http://localhost:{args.port}"
    caps = {"capabilities": {"alwaysMatch": {"webkitgtk:browserOptions": {"args": []}}}}
    sid = rq(base, "POST", "/session", caps)["value"]["sessionId"]
    try:
        rq(base, "POST", f"/session/{sid}/window/rect",
           {"width": args.width, "height": args.height, "x": 0, "y": 0})
        rq(base, "POST", f"/session/{sid}/url", {"url": args.url})
        time.sleep(args.settle)
        if args.open_selector:
            clicked = rq(base, "POST", f"/session/{sid}/execute/sync", {
                "script": "const el = document.querySelector(arguments[0]);"
                          " if (!el) return false; el.click(); return true;",
                "args": [args.open_selector],
            })["value"]
            # Say so rather than carry on: a selector that matches nothing means the
            # rest of the run reports on a page that never opened, which is the
            # exact shape of every false green this harness has produced.
            if not clicked:
                print(f"open selector matched nothing: {args.open_selector}", file=sys.stderr)
                sys.exit(3)
            time.sleep(args.after)
        if args.inject:
            script = open(args.inject).read()
            res = rq(base, "POST", f"/session/{sid}/execute/sync",
                     {"script": script, "args": []})
            print("inject result:", res.get("value"), file=sys.stderr)
            time.sleep(args.after)
        shot = rq(base, "GET", f"/session/{sid}/screenshot")["value"]
        with open(args.out, "wb") as f:
            f.write(base64.b64decode(shot))
        print("wrote", args.out)
    finally:
        try:
            rq(base, "DELETE", f"/session/{sid}")
        except Exception:
            pass


if __name__ == "__main__":
    main()
