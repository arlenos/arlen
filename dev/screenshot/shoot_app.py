#!/usr/bin/env python3
"""Test Layer 1b full-app WebDriver client: launch a real Tauri binary through an
already-running tauri-driver, optionally type a command, and save a screenshot.

Unlike shoot.py (which loads a URL in WebKitWebDriver, isolating the frontend),
this drives the ACTUAL app - the Rust backend and the webview together - so it
verifies the whole thing (IPC + render), e.g. that terminal command output shows.

shoot-app.sh starts tauri-driver under Xvfb. Stdlib only, no venv.
"""
import argparse
import base64
import json
import sys
import time
import urllib.request

# WebDriver key code for Enter (U+E007), used to submit a typed command.
ENTER = ""


def rq(base, method, path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(
        base + path, data=data, method=method,
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.load(r)


def type_keys(base, sid, text):
    """Send `text` to the focused element as a WebDriver key-action sequence."""
    actions = []
    for ch in text:
        actions.append({"type": "keyDown", "value": ch})
        actions.append({"type": "keyUp", "value": ch})
    rq(base, "POST", f"/session/{sid}/actions",
       {"actions": [{"type": "key", "id": "kbd", "actions": actions}]})


def find_element(base, sid, css):
    """Find one element by CSS selector; return its W3C element reference."""
    res = rq(base, "POST", f"/session/{sid}/element",
             {"using": "css selector", "value": css})["value"]
    # W3C returns a single-entry dict {"element-...": "<reference>"}.
    return list(res.values())[0]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--app", required=True, help="path to the Tauri app binary")
    ap.add_argument("--out", required=True)
    ap.add_argument("--port", type=int, default=4444)
    ap.add_argument("--settle", type=float, default=3.0,
                    help="seconds to wait for the app to come up")
    ap.add_argument("--type", default=None,
                    help="text to type into the input (Enter appended)")
    ap.add_argument("--selector", default=None,
                    help="CSS selector of the input to type into")
    args = ap.parse_args()

    base = f"http://localhost:{args.port}"
    caps = {"capabilities": {"alwaysMatch": {"tauri:options": {"application": args.app}}}}
    sid = rq(base, "POST", "/session", caps)["value"]["sessionId"]
    try:
        time.sleep(args.settle)
        if args.type:
            # Type the command into the composer with the canonical WebDriver
            # Element Send Keys endpoint: find the input, then send the text plus
            # Enter. Send Keys produces real key events the framework's handlers
            # see (unlike raw Actions, where Enter does not reliably map to
            # `event.key === "Enter"`, or a synthetic dispatch that Svelte 5's
            # event delegation may miss). Falls back to the first text field.
            sel = args.selector or "#terminal-composer-input"
            try:
                eid = find_element(base, sid, sel)
            except Exception:
                eid = find_element(base, sid,
                                   "textarea,input[type=text],input:not([type])")
            rq(base, "POST", f"/session/{sid}/element/{eid}/value",
               {"text": args.type + ENTER})
            print("sent keys to", sel, file=sys.stderr)
            time.sleep(2.5)
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
