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
    args = ap.parse_args()

    base = f"http://localhost:{args.port}"
    caps = {"capabilities": {"alwaysMatch": {"webkitgtk:browserOptions": {"args": []}}}}
    sid = rq(base, "POST", "/session", caps)["value"]["sessionId"]
    try:
        rq(base, "POST", f"/session/{sid}/window/rect",
           {"width": args.width, "height": args.height, "x": 0, "y": 0})
        rq(base, "POST", f"/session/{sid}/url", {"url": args.url})
        time.sleep(args.settle)
        if args.inject:
            script = open(args.inject).read()
            res = rq(base, "POST", f"/session/{sid}/execute/sync",
                     {"script": script, "args": []})
            print("inject result:", res.get("value"), file=sys.stderr)
            time.sleep(0.8)
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
