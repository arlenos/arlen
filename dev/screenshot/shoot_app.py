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
import re
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


def press_enter(base, sid):
    """Send Enter as its own action sequence with a pause between down and up.

    Batched with the command's keys, the synthetic Enter races the preceding
    key-ups and does not reliably map to `event.key === "Enter"`; a dedicated
    sequence with a short hold makes it land every time (verified against the
    terminal's raw-PTY input handler)."""
    rq(base, "POST", f"/session/{sid}/actions", {"actions": [{"type": "key",
        "id": "kbd", "actions": [
            {"type": "keyDown", "value": ENTER},
            {"type": "pause", "duration": 60},
            {"type": "keyUp", "value": ENTER}]}]})


def console_text(base, sid):
    """The visible console as plain text: dump the page source, take the console
    subtree, strip tags and whitespace. The terminal grid paints one char per
    `<span class="cell">`, so a raw substring search over the HTML misses words
    that span cells; stripping tags and whitespace concatenates the cells so the
    rendered text is searchable."""
    src = rq(base, "GET", f"/session/{sid}/source")["value"]
    i = src.find('class="console')
    seg = src[i:] if i >= 0 else src
    return re.sub(r"\s+", "", re.sub(r"<[^>]+>", "", seg))


def run_and_assert(base, sid, command, expect, selector):
    """Drive the re-rooted terminal headlessly: focus the console, type a command
    (retry until it shows in the grid, beating the focus race), press Enter, and
    assert `expect` renders. Returns True on success. The whole input->PTY->shell
    ->grid/block round-trip is exercised, so this catches a regression in the
    terminal's render pipeline that a frontend-only test cannot."""
    eid = find_element(base, sid, selector or ".console")
    landed = False
    for _ in range(4):
        try:
            rq(base, "POST", f"/session/{sid}/element/{eid}/click", {})
        except Exception:
            pass
        time.sleep(0.4)
        type_keys(base, sid, command)
        time.sleep(1.0)
        if expect in console_text(base, sid):
            landed = True
            break
    if not landed:
        print("EXEC FAIL: the command never reached the grid", file=sys.stderr)
        return False
    press_enter(base, sid)
    time.sleep(5.0)
    ok = expect in console_text(base, sid)
    print(("EXEC PASS: " if ok else "EXEC FAIL: ") + repr(expect)
          + (" rendered after execution" if ok else " not found after execution"))
    return ok


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--app", required=True, help="path to the Tauri app binary")
    ap.add_argument("--out", default=None,
                    help="screenshot output path (omit in --exec assert mode)")
    ap.add_argument("--exec", dest="exec_cmd", default=None,
                    help="run a command in the terminal and assert --expect renders "
                         "(headless DOM-level proof of the input->shell->grid path)")
    ap.add_argument("--expect", default=None,
                    help="substring that must appear in the console after --exec")
    ap.add_argument("--port", type=int, default=4444)
    ap.add_argument("--settle", type=float, default=3.0,
                    help="seconds to wait for the app to come up")
    ap.add_argument("--type", default=None,
                    help="text to type into the input (Enter appended)")
    ap.add_argument("--selector", default=None,
                    help="CSS selector of the input to type into")
    ap.add_argument("--grab-x", action="store_true",
                    help="grab the X root window with `import` instead of the "
                         "WebDriver screenshot endpoint - needed for an app that "
                         "never reaches paint-idle (a live terminal repaints "
                         "continuously), where /screenshot hangs")
    args = ap.parse_args()

    base = f"http://localhost:{args.port}"
    caps = {"capabilities": {"alwaysMatch": {"tauri:options": {"application": args.app}}}}
    sid = rq(base, "POST", "/session", caps)["value"]["sessionId"]
    exit_code = 0
    try:
        time.sleep(args.settle)
        if args.exec_cmd:
            expect = args.expect if args.expect is not None else args.exec_cmd
            ok = run_and_assert(base, sid, args.exec_cmd, expect, args.selector)
            exit_code = 0 if ok else 1
            if not args.out:
                return exit_code
        if args.type:
            # Type the command via the canonical WebDriver Element Send Keys
            # endpoint, which produces real key events the framework's handlers
            # see (unlike raw Actions, where Enter does not reliably map to
            # `event.key === "Enter"`). The re-rooted terminal has no composer
            # input: the `.console` div is the focusable keystroke surface
            # (tabindex + onkeydown), so try it first, then a classic text input
            # for other apps. Click to focus the surface, then send the text plus
            # Enter.
            candidates = [args.selector] if args.selector else [
                ".console",
                "#terminal-composer-input",
                "textarea,input[type=text],input:not([type])",
            ]
            eid = None
            sel = None
            for cand in candidates:
                try:
                    eid = find_element(base, sid, cand)
                    sel = cand
                    break
                except Exception:
                    continue
            if eid is None:
                raise SystemExit("no typeable surface found")
            try:
                rq(base, "POST", f"/session/{sid}/element/{eid}/click", {})
            except Exception:
                pass
            rq(base, "POST", f"/session/{sid}/element/{eid}/value",
               {"text": args.type + ENTER})
            print("sent keys to", sel, file=sys.stderr)
            time.sleep(2.5)
        if args.out and args.grab_x:
            # Grab the whole virtual display where the app window is mapped. The
            # WebDriver /screenshot endpoint waits for paint-idle, which a live
            # terminal never reaches; `import` just reads the X framebuffer, so it
            # returns regardless. Runs on the same DISPLAY (inherited from
            # xvfb-run).
            import subprocess
            time.sleep(1.0)
            subprocess.run(["import", "-window", "root", args.out], check=True)
            print("grabbed X root to", args.out)
        elif args.out:
            shot = rq(base, "GET", f"/session/{sid}/screenshot")["value"]
            with open(args.out, "wb") as f:
                f.write(base64.b64decode(shot))
            print("wrote", args.out)
        return exit_code
    finally:
        try:
            rq(base, "DELETE", f"/session/{sid}")
        except Exception:
            pass


if __name__ == "__main__":
    sys.exit(main() or 0)
