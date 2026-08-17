#!/usr/bin/env python3
"""Test Layer 1b full-app WebDriver client: launch a real Tauri binary through an
already-running tauri-driver, optionally type a command, and save a screenshot.

Unlike shoot.py (which loads a URL in WebKitWebDriver, isolating the frontend),
this drives the ACTUAL app - the Rust backend and the webview together - so it
verifies the whole thing (IPC + render), e.g. that terminal command output shows.

shoot-app.sh starts tauri-driver under Xvfb. Stdlib only, no venv.

**Two things this cannot see, both of which look like a defect if you forget.**

Titlebar controls are NOT in the webview. An app declares them and the compositor
renders and handles them (`arlen-titlebar-v1`), so under this harness - which has
no compositor - a declared button appears as inert text in a `header` and no
selector will find anything clickable. `Focus Now` in the text editor cost three
probes and an accessibility scare on 17 August before that was the answer. A
driven-UI check that finds no button in the titlebar has found nothing, not a bug.

A label may be split across adjacent text nodes, so `Focus` and `Now` live in
separate nodes and `textContent` is what joins them. Match against an element's
`textContent`, never per text node, and never against `innerText` - that one is
layout-derived, so whether the split shows as `Focus Now` or `Focus\\nNow` changes
between renders of the same page. All three of my failed probes that day were one
of these two.
"""
import argparse
import base64
import json
import re
from pathlib import Path
import sys
import time
import urllib.request
import urllib.error

# WebDriver key code for Enter (U+E007), used to submit a typed command.
ENTER = ""


def rq(base, method, path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(
        base + path, data=data, method=method,
        headers={"Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            return json.load(r)
    except urllib.error.HTTPError as e:
        # WebDriver puts the reason in the response body, and urllib throws that
        # body away - so a failure used to surface as a bare "HTTP Error 500"
        # with a traceback and nothing to act on. Reading it costs one line and
        # turns every future failure here into something diagnosable.
        detail = e.read().decode(errors="replace").strip()
        raise SystemExit(f"{method} {path} -> HTTP {e.code}\n{detail}") from None


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
    # `.console` on purpose, and NOT the helper textarea the send-keys path
    # needs: this path CLICKS to focus (which is what hands xterm the focus) and
    # then sends key ACTIONS, which xterm receives either way. Verified working
    # after the block-mode cutover, so leave it alone - the two paths differ in
    # mechanism, not by oversight.
    # `console_text` returns the grid with every run of whitespace removed, so
    # anything compared against it has to be squashed the same way.
    squash = lambda s: re.sub(r"\s+", "", s)
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
        # Wait for the COMMAND to appear, not for the expectation. This asked for
        # `expect`, which is a different question and answered wrong twice over: a
        # multi-word expectation never matches the squashed grid, so the loop
        # timed out and reported "the command never reached the grid" about a
        # command sitting in plain sight, and a single-word one that happens to be
        # in the command made this the only test that ran.
        if squash(command) in console_text(base, sid):
            landed = True
            break
    if not landed:
        print("EXEC FAIL: the command never reached the grid", file=sys.stderr)
        return False
    press_enter(base, sid)
    time.sleep(5.0)
    # Look for `expect` in the OUTPUT, which means not in the echoed command.
    #
    # This used to be `expect in console_text(...)` - the same test line 108 uses
    # to decide the command was TYPED. Once the command is on screen that test is
    # already satisfied, so the post-execution assertion passed on the prompt
    # rather than on any result. Measured on 16 August:
    # `SHOOT_EXEC='true # ZZMARKER' SHOOT_EXPECT=ZZMARKER` produces no output
    # whatsoever and reported "EXEC PASS: rendered after execution". Every check
    # of the form `echo foo` / expect `foo` was passing on the echo of the word
    # `foo` in the command line, which is to say the terminal's one end-to-end
    # verification could not fail for the reason it existed.
    #
    # Dropping every line that carries the command text leaves the output. A
    # command whose own text appears in its output loses that line too, which
    # costs a false negative and never a false pass - the direction this has to
    # fail in.
    # `console_text` squashes every run of whitespace away, so both sides have to
    # be squashed too. That is the second half of this defect and it fails the
    # other way: `SHOOT_EXPECT='No such file'` could never match, because the text
    # being searched reads `Nosuchfile`. One check that cannot fail for
    # single-word expectations found in the command, and cannot pass for
    # multi-word ones. Both directions are fixed here.
    out = console_text(base, sid).replace(squash(command), "")
    ok = squash(expect) in out
    print(("EXEC PASS: " if ok else "EXEC FAIL: ") + repr(expect)
          + (" rendered after execution" if ok else " not found in the output"))
    return ok


def warn_if_error_page(base, sid):
    """Fail if what got photographed was the webview's own error page.

    A debug Tauri binary loads `build.devUrl`, not the bundled frontend, so a
    freshly built app with no dev server behind it renders "Could not connect to
    localhost: Connection refused" - a valid PNG, exit code 0, and no app in it.
    That happened on the text editor's first launch shot and the file gave no
    hint; it looked like a screenshot until somebody opened it.

    This reads the DOM rather than the pixels, so it catches the message before
    the eye has to. It does not judge whether the app rendered CORRECTLY - only
    that it is the app rather than a failure to reach it.

    The probes are searched across the whole page source, so an app that legitimately
    DISPLAYS one of these phrases is reported as an error page - a terminal showing
    `curl: Connection refused`, most obviously, which is a plausible thing to want a
    screenshot of. Left as it is on purpose: the cost is a re-run with different
    text, while narrowing the search risks missing the case this exists for, and
    this check has to fail toward refusing a shot rather than toward blessing one.
    """
    try:
        html = rq(base, "GET", f"/session/{sid}/source")["value"]
    except Exception:
        return 0  # no source endpoint: not a reason to fail a good shot
    for probe in ("Connection refused", "ERR_CONNECTION", "Unable to load page",
                  "did not respond"):
        if probe in html:
            print(
                f"SHOT IS AN ERROR PAGE ({probe!r}): the binary loaded its devUrl "
                f"and nothing was serving it. Either build with --release, or run "
                f"the app's `vite preview` on the port in its tauri.conf.json.",
                file=sys.stderr,
            )
            return 1
    return 0


def wrong_app(binary: str, loaded: str) -> str | None:
    """Say so when the window loaded a URL that is not this app's `devUrl`.

    A debug binary carries the `devUrl` it was BUILT with, so one that predates a
    port change opens whichever app now serves the old port - and the shot looks
    entirely normal. On 16 August a terminal binary from a week earlier loaded the
    screenshot app, and the run reported a clean console for an app it never
    opened. Printing the URL was not enough; it has to be compared.

    Only the dev-server case is judged: a release build serves `tauri://localhost`
    and has no port to disagree about, and a driver that cannot answer leaves this
    alone rather than failing a shot for the wrong reason.
    """
    if not loaded.startswith("http://localhost:"):
        return None
    root = Path(__file__).resolve().parents[2]
    name = Path(binary).name
    for cargo in (root / "apps").glob("*/src-tauri/Cargo.toml"):
        if not re.search(rf'^name\s*=\s*"{re.escape(name)}"', cargo.read_text(), re.M):
            continue
        conf = cargo.parent / "tauri.conf.json"
        if not conf.is_file():
            return None
        want = json.loads(conf.read_text()).get("build", {}).get("devUrl", "")
        if not want or loaded.rstrip("/").startswith(want.rstrip("/")):
            return None
        return (
            f"{cargo.parent.parent.name} is configured for {want} but the window "
            f"loaded {loaded} - this binary was built against an older devUrl"
        )
    return None


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
    ap.add_argument("--inject", action="append", default=None,
                    help="path to a JS file to run in the page; its return value "
                         "is printed as `inject result: <value>`. The way to ask "
                         "the running app a question about its own DOM, which is "
                         "the only place a Tauri command's output can be seen. "
                         "Repeatable, with a pause between: the first can move the "
                         "app somewhere (a route, an open dialog) and the second "
                         "ask about what is there, which one call cannot do "
                         "because navigating discards the script's return")
    ap.add_argument("--inject-settle", type=float, default=2.5,
                    help="seconds between repeated --inject runs")
    ap.add_argument("--grab-x", action="store_true",
                    help="grab the X root window with `import` instead of the "
                         "WebDriver screenshot endpoint - needed for an app that "
                         "never reaches paint-idle (a live terminal repaints "
                         "continuously), where /screenshot hangs")
    ap.add_argument("--app-arg", action="append",
                    help="argument passed to the app binary (repeatable), e.g. a "
                         "file path for an app launched on a file")
    args = ap.parse_args()

    base = f"http://localhost:{args.port}"
    # tauri-driver forwards `args` to the binary, which is how an app that takes a
    # file on the command line (the viewers, the text editor, anything reached by a
    # desktop entry's `%f`) can be photographed opening a real file rather than its
    # no-argument state.
    opts = {"application": args.app}
    if args.app_arg:
        opts["args"] = args.app_arg
    caps = {"capabilities": {"alwaysMatch": {"tauri:options": opts}}}
    sid = rq(base, "POST", "/session", caps)["value"]["sessionId"]
    exit_code = 0
    try:
        time.sleep(args.settle)

        # WHICH PAGE DID THE WINDOW ACTUALLY LOAD?
        #
        # A debug binary loads the `devUrl` baked in AT BUILD TIME, so a binary
        # older than a port change opens whatever now serves the old port - another
        # app's dev server, in another app's window, with no error anywhere. On
        # 16 August a terminal binary from 9 August loaded the screenshot app and a
        # verification run reported a clean console for an app it never opened.
        #
        # The URL is one line and settles it, so it is always printed rather than
        # asked for.
        try:
            loaded = rq(base, "POST", f"/session/{sid}/execute/sync",
                        {"script": "return location.href;", "args": []})["value"]
            print(f"loaded url: {loaded}")
            wrong = wrong_app(args.app, loaded)
            if wrong:
                print(f"WRONG APP: {wrong}")
                print("Rebuild the binary; a shot of the wrong app proves nothing.")
                return 1
        except Exception as e:  # a driver that cannot answer must not fail the shot
            print(f"loaded url: unknown ({e})")

        # BEFORE the capture, not after it. This used to run once the PNG was
        # already written and "wrote <path>" already printed, so a run against a
        # dead dev server left a connection-refused image on disk - the exact
        # thing this function's own note calls out, "it looked like a screenshot
        # until somebody opened it". A nonzero exit does not delete a file, and
        # the file is what the next reader finds.
        if args.out:
            err = warn_if_error_page(base, sid)
            if err:
                print("No screenshot written.", file=sys.stderr)
                return err

        if args.exec_cmd:
            expect = args.expect if args.expect is not None else args.exec_cmd
            ok = run_and_assert(base, sid, args.exec_cmd, expect, args.selector)
            exit_code = 0 if ok else 1
            if not args.out:
                return exit_code
        for n, path in enumerate(args.inject or []):
            if n:
                time.sleep(args.inject_settle)
            with open(path) as f:
                script = f.read()
            # Same endpoint and the same `inject result:` line as `shoot.py`, so
            # the scanner reads one shape whichever harness produced it.
            out = rq(base, "POST", f"/session/{sid}/execute/sync",
                     {"script": script, "args": []})["value"]
            print(f"inject result: {out}")

        if args.type:
            # Type the command via the canonical WebDriver Element Send Keys
            # endpoint, which produces real key events the framework's handlers
            # see (unlike raw Actions, where Enter does not reliably map to
            # `event.key === "Enter"`).
            #
            # ORDER MATTERS, and the first entry is the one this got wrong. The
            # note here used to say the terminal's `.console` div is the focusable
            # keystroke surface, which was true of the hand-rolled grid; after the
            # block-mode cutover xterm.js owns the terminal and keystrokes go to
            # its hidden helper textarea. `.console` still EXISTS as the container,
            # so the old first guess kept matching and then failed the send with
            # "element not interactable" - the harness's own documented use case,
            # typing a terminal command, silently unavailable because a selector
            # outlived the thing it described.
            candidates = [args.selector] if args.selector else [
                ".xterm-helper-textarea",
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
