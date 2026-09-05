#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Render a URL at a chosen CSS viewport width, and prove the width was reached.

`shoot.py` asks WebKitWebDriver for a window size and is refused in silence: the
driver answers 200, echoes the rectangle back, and `window.innerWidth` stays 372
whatever you ask for. 372 CSS px is a phone, so every screenshot that loop has
produced is of the narrow layout, and none of them said so. That is the reason
this exists.

**How the width is actually reached, because it is not obvious and three
plausible routes do not work.** Measured 9 August on this machine:

  - `set_default_size(1440, 960)` yields a 372 px window. Same with and without
    openbox, same with a plain `Gtk.Label` in place of the WebView, so it is
    neither the window manager nor WebKit.
  - `maximize()` changes nothing.
  - The Xvfb screen size changes nothing either: 1280x900, 1600x1200 and
    2560x1440 all produce the same window. GTK reports one fixed monitor,
    1504x1002 at scale 2, whatever the server was started with.
  - `fullscreen()` DOES work, and gives the full 1504x1002.

So the window goes fullscreen to get a real desktop-sized surface, and the exact
width comes from zoom: at `zoom = surface_width / wanted`, the page lays out at
precisely `wanted` CSS px. Verified rather than assumed - 1024, 1280 and 1440
each come back exactly. Everything is drawn proportionally smaller or larger, so
the PNG is a faithful picture of that layout at that width, which is the thing
being checked.

**The viewport HEIGHT follows from the width; it is not a second knob.** The
surface has one aspect ratio and zoom preserves it, so asking for a height would
be a control that cannot work. The height reached is reported instead. The
capture is the full document either way.

Engine note: this draws with WebKitGTK 6.0 (GTK 4), while the Tauri apps link
webkit2gtk-4.1. Same engine, different API generation - close enough to judge a
layout, not the identical build. For a shot of a real app window as the app
itself draws it, use `shoot-app.sh`, which drives the actual binary.

Usage, under a display:
  xvfb-run -a --server-args="-screen 0 1600x1200x24" \
    dev/screenshot/render-wide.py --url http://localhost:5300/printers \
      --out shot.png --width 1280 --require-width 1280

`--require-width` refuses BEFORE capturing, so a run that could not reach desktop
width leaves nothing behind to be mistaken later for a render of what it was
asked for.
"""
import argparse
import json
import os
import pathlib
import sys

# CUT THE HOST SESSION OFF BEFORE GTK IS IMPORTED. Not optional and not a caller's
# job, because getting it wrong puts a fullscreen window on the developer's real
# screen while they are working - which is what happened on 25 August, when this
# was run directly under `xvfb-run` instead of through `shoot.sh`.
#
# `xvfb-run` sets DISPLAY and nothing else. An inherited WAYLAND_DISPLAY stays
# valid, GTK 4 prefers the Wayland backend when it sees one, and the window opens
# on the session rather than the Xvfb - so the shot is of the wrong display and
# the developer is interrupted. `shoot.sh` has unset these since 15 August and
# says exactly why; the mistake was that this file relied on being called through
# it. It does not any more: a direct invocation is now as safe as a wrapped one.
#
# It must happen before `import gi`, since the backend is chosen at import time.
os.environ.pop("WAYLAND_DISPLAY", None)
os.environ["GDK_BACKEND"] = "x11"
if not os.environ.get("DISPLAY"):
    # No X server to draw on and no Wayland left to fall back to. Refusing is the
    # only safe answer: the fallback would be the session this just disconnected.
    sys.stderr.write(
        "refusing: no DISPLAY. Run under `xvfb-run -a --server-args=\"-screen 0"
        " 1600x1200x24\"` so the window has an off-screen server to draw on.\n"
    )
    raise SystemExit(3)

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("WebKit", "6.0")
from gi.repository import GLib, Gtk, WebKit  # noqa: E402

# Below this the zoom needed to reach the requested width would shrink text past
# legibility in the PNG, and a picture nobody can read is not a verification.
MIN_ZOOM = 0.2
MAX_ZOOM = 6.0


# axe-core ships with the kit's dev dependencies; the same engine its own jsdom
# gate uses, so a finding here and a finding there mean the same thing.
AXE_PATH = pathlib.Path(__file__).resolve().parents[2] / "sdk/ui-kit/node_modules/axe-core/axe.min.js"


class Render:
    """One window, one page, one measured capture."""

    def __init__(self, args):
        self.axe_failures = 0
        self.args = args
        self.status = 1
        self.app = Gtk.Application(application_id="dev.arlen.render-wide")
        self.app.connect("activate", self.on_activate)

    def run(self):
        self.app.run([])
        return self.status

    def fail(self, message, status):
        print(message, file=sys.stderr)
        self.status = status
        self.app.quit()

    # A Tauri runtime that is PRESENT and answers every command with a refusal.
    #
    # Without it this harness can only render the no-runtime path, and for a Tauri
    # app that is the browser preview - `tauriAvailable` is false, so every
    # `if (!tauriAvailable) { fixture }` guard fires and what gets photographed is
    # the demo the author put there on purpose. The path a person actually meets
    # is the other one: the runtime is there from the moment the webview loads, and
    # the BACKEND fails. That is where the fixture-on-failure defects live, and
    # nothing here could reach it.
    #
    # The refusal is deliberately unnameable. An app that decodes named problems
    # falls through to its escape branch, which is exactly the branch worth looking
    # at: it is the one that ends up quoting machinery at a person.
    STUB_HOST = """
      window.__TAURI_INTERNALS__ = {
        invoke: function (cmd) {
          return Promise.reject('stub-host: no backend behind this window (' + cmd + ')');
        },
        transformCallback: function (cb) { return cb; },
        metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
      };
      window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
    """

    def host_source(self):
        """The runtime script to inject, or None for a page with no runtime.

        Two knobs, because there turned out to be a THIRD state neither of them
        could reach. `--stub-host` refuses everything, so a surface whose list
        comes from the backend has no list and therefore no row to press; no
        runtime at all takes the fixture path, where the store's own
        `if (tauriAvailable)` guard means an action can never be refused. The
        message shown when one action out of many is refused was unreachable in
        both, which is how it went unlooked-at while being the ordinary case on
        that surface - End process on something you do not own.

        `--host-script` is the general answer: a file that installs whatever
        runtime the shot needs, answering some commands and refusing others. It
        replaces the stub rather than layering on it, so what a run injected is
        one file somebody can read.
        """
        if self.args.host_script:
            return pathlib.Path(self.args.host_script).read_text(encoding="utf-8")
        if self.args.stub_host:
            return self.STUB_HOST
        return None

    def on_activate(self, app):
        self.win = Gtk.ApplicationWindow(application=app)
        host = self.host_source()
        if host is not None:
            ucm = WebKit.UserContentManager()
            ucm.add_script(
                WebKit.UserScript.new(
                    host,
                    WebKit.UserContentInjectedFrames.TOP_FRAME,
                    WebKit.UserScriptInjectionTime.START,
                    None,
                    None,
                )
            )
            self.view = WebKit.WebView(user_content_manager=ucm)
        else:
            self.view = WebKit.WebView()
        self.view.set_hexpand(True)
        self.view.set_vexpand(True)
        self.win.set_child(self.view)
        # The only sizing call that this GTK honours - see the note at the top.
        self.win.fullscreen()
        self.win.present()
        self.view.connect("load-changed", self.on_load)
        # A page that could not be fetched still "finishes loading": WebKit swaps
        # in its own error document and every later step runs against THAT. It
        # cost a whole axe report - two confident violations about a page whose
        # `app.html` plainly had both, because the dev server was not up and axe
        # had audited WebKit's error page. A checker pointed at nothing answers
        # about nothing, which is the same rule `--open` already follows when its
        # selector matches no element.
        self.view.connect("load-failed", self.on_load_failed)
        self.view.load_uri(self.args.url)
        # A page that never finishes loading would otherwise hang the harness
        # forever, which in an unattended sweep looks like a machine that died.
        GLib.timeout_add_seconds(self.args.timeout, self.on_timeout)

    def on_load_failed(self, view, event, failing_uri, error):
        self.fail(f"refusing: {failing_uri} did not load ({error.message}). "
                  f"Nothing was captured; anything measured here would be about "
                  f"WebKit's error page.", 9)
        return True  # handled: do not let the error document render

    def on_timeout(self):
        self.fail(f"gave up after {self.args.timeout}s:"
                  f" the page never finished loading", 5)
        return False

    def on_load(self, view, event):
        if event == WebKit.LoadEvent.FINISHED:
            GLib.timeout_add(int(self.args.settle * 1000), self.set_width)

    def set_width(self):
        surface = self.view.get_width()
        if surface <= 0:
            self.fail("the window was never given a size, so there is nothing"
                      " to scale from", 6)
            return False
        zoom = surface / self.args.width
        if not MIN_ZOOM <= zoom <= MAX_ZOOM:
            self.fail(f"reaching {self.args.width}px from a {surface}px surface"
                      f" needs a zoom of {zoom:.2f}, outside"
                      f" {MIN_ZOOM}-{MAX_ZOOM}. No screenshot written.", 4)
            return False
        self.view.set_zoom_level(zoom)
        # Zoom relays out the page; measuring in the same tick reports the old
        # width and would make the check pass on a shot that had not reflowed.
        GLib.timeout_add(int(self.args.reflow * 1000), self.measure)
        return False

    def measure(self):
        self.view.evaluate_javascript(
            "JSON.stringify([window.innerWidth, window.innerHeight,"
            " window.devicePixelRatio])",
            -1, None, None, None, self.on_measured)
        return False

    def on_measured(self, view, result):
        try:
            css_w, css_h, dpr = json.loads(
                view.evaluate_javascript_finish(result).to_string())
        except Exception as e:  # noqa: BLE001 - any failure here means no verdict
            self.fail(f"could not measure the viewport: {e}", 6)
            return
        # Printed on every run, pass or fail: a PNG carries no record of the
        # viewport it was taken at, and the reader's only other source is the
        # number they typed on the command line.
        print(f"viewport: {css_w}x{css_h} css px at dpr {dpr:.2f}"
              f" (asked for width {self.args.width})", file=sys.stderr)
        if self.args.require_width is not None and css_w < self.args.require_width:
            self.fail(f"refusing: needed a viewport of at least"
                      f" {self.args.require_width}px and got {css_w}px."
                      f" No screenshot written.", 4)
            return
        if self.args.type:
            self.type_into()
            return
        if self.args.open:
            self.pending = list(self.args.open)
            self.click_open()
            return
        self.finish()

    def finish(self):
        """The last step, whatever the run was for: probe, or audit, or shoot.

        This is a function rather than three branches at the load-finished point
        because `--probe` used to be the FIRST of them, and returned before the
        click chain ever started. A probe combined with `--open` therefore
        reported the page as it was BEFORE the click, printed it without a word
        about that, and exited 0.

        Measured on 16 August against the knowledge app with no backend: clicking
        Pause and asking for the alert text answered "no alert", which reads as a
        control that fails silently. The alert is there; the question was asked
        one step too early. A tool that answers a different question than the one
        typed, and looks right doing it, is worse than one that refuses - the
        report it produces is a bug that does not exist, and the fix for it lands
        in an app that was already correct.
        """
        if self.args.probe or self.args.probe_file:
            self.run_probe()
        elif self.args.axe:
            self.run_axe()
        else:
            self.snapshot()
        return False

    def run_probe(self):
        """Print one expression's value from the rendered page, and shoot nothing.

        Every DOM question so far has been answered by grepping the source, which
        works until the markup in question is assembled from a shared component -
        then the source says what each part contributes and nothing says what the
        page IS. axe reported a `ul` with the wrong children in two apps and named
        neither; one app's own markup was clean, so the answer was only ever going
        to come from the render.

        Deliberately narrow in what it returns, not in what it may ask: the value
        is stringified, so a probe reports a finding rather than driving the page.
        """
        # A FILE IS A BODY, an inline argument is an expression. The three render
        # probes are multi-statement scripts ending in `return`, so passing one
        # through --probe fails with "Unexpected keyword 'const'" - which is how
        # the failure surfaces went unprobed until 6 September: `shoot.sh` can
        # run them but cannot install a host, and this can install a host but
        # could not run them. Wrapped rather than evaluated, so the same file
        # works both ways.
        if self.args.probe_file:
            body = pathlib.Path(self.args.probe_file).read_text(encoding="utf-8")
            js = "String((() => {\n" + body + "\n})())"
        else:
            js = f"String((() => {{ return ({self.args.probe}); }})())"
        self.view.evaluate_javascript(js, -1, None, None, None, self.on_probe)

    def on_probe(self, view, result):
        try:
            print(view.evaluate_javascript_finish(result).to_string())
        except Exception as e:  # noqa: BLE001
            self.fail(f"probe failed: {e}", 10)
            return
        self.status = 0
        self.app.quit()

    def run_axe(self):
        """Inject axe-core into the rendered page and report what it finds.

        The kit's own axe gate runs the primitives under jsdom, which has no
        layout - so it can check roles and names and cannot check anything that
        needs a box. This runs the SAME engine against the real WebKit render of
        a real app page, where an app composes those primitives into a surface
        the kit never sees: a page with no landmark, a dialog with no accessible
        name, an icon-only button the app added itself.

        The result goes to stdout as one line per violation rather than into the
        PNG, because a screenshot cannot show a missing name.
        """
        try:
            axe_src = AXE_PATH.read_text(encoding="utf-8")
        except OSError as e:
            self.fail(f"axe-core not readable at {AXE_PATH}: {e}", 8)
            return
        # `color-contrast` stays ON here, unlike the kit's jsdom run: this render
        # has real layout, which is the whole reason to check a11y out here.
        #
        # The result is stashed on `window` and read by a second evaluation
        # rather than returned: `axe.run` is a promise, and WebKit's
        # `evaluate_javascript` cannot marshal one back ("Unsupported result
        # type"). So this kicks it off and `poll_axe` waits for the answer.
        js = (
            axe_src
            + "\n;window.__axe = null; window.__axeErr = null;"
            " axe.run(document, {resultTypes:['violations']}).then("
            "  r => { window.__axe = JSON.stringify(r.violations.map(v => ({id: v.id,"
            "    impact: v.impact, help: v.help, n: v.nodes.length,"
            "    first: v.nodes[0] && v.nodes[0].target.join(' ')}))); },"
            "  e => { window.__axeErr = String(e); });"
            # The evaluation's own value must not be the promise: WebKit cannot
            # marshal one, and the whole script is one expression list, so the
            # last one is what comes back. A string ends it.
            " 'started';"
        )
        self.axe_waited = 0
        self.view.evaluate_javascript(js, -1, None, None, None, self.on_axe_started)

    def on_axe_started(self, view, result):
        try:
            view.evaluate_javascript_finish(result)
        except Exception as e:  # noqa: BLE001
            self.fail(f"axe could not be injected: {e}", 8)
            return
        GLib.timeout_add(200, self.poll_axe)

    def poll_axe(self):
        self.axe_waited += 200
        if self.axe_waited > 30000:
            self.fail("axe did not finish within 30s", 8)
            return False
        self.view.evaluate_javascript(
            "window.__axeErr ? 'ERR:' + window.__axeErr : (window.__axe || '')",
            -1, None, None, None, self.on_axe)
        return False

    def on_axe(self, view, result):
        try:
            payload = view.evaluate_javascript_finish(result).to_string()
        except Exception as e:  # noqa: BLE001
            self.fail(f"axe run failed: {e}", 8)
            return
        if payload.startswith("ERR:"):
            self.fail(f"axe threw in the page: {payload[4:]:.200}", 8)
            return
        if not payload:
            GLib.timeout_add(200, self.poll_axe)
            return
        try:
            found = json.loads(payload) if payload and payload != "null" else []
        except (json.JSONDecodeError, TypeError):
            self.fail(f"axe returned something unreadable: {payload!r:.200}", 8)
            return
        if not found:
            print("axe: no violations")
        else:
            print(f"axe: {len(found)} violation(s)")
            for v in found:
                print(f"  {v['id']} ({v['impact']}): {v['help']} [{v['n']}x] -> {v.get('first')}")
        self.axe_failures = len(found)
        self.snapshot()

    def type_into(self):
        """Put text in a field the way a person does, then let it settle.

        Not a convenience. A search surface's most important copy is the line
        under NO results, and reaching it needs a query in the box - so until
        this existed, no screenshot in this loop had ever contained a typed one,
        and the launcher's empty state had never been looked at.

        The value is set through the native setter and followed by a real `input`
        event, because a framework listening for that (cmdk, Svelte's bind) does
        not see a plain assignment to `.value`.
        """
        sel, _, text = self.args.type.partition("::")
        js = (
            "(() => { const el = document.querySelector(%s);"
            " if (!el) return 'missing';"
            " const set = Object.getOwnPropertyDescriptor("
            "   Object.getPrototypeOf(el), 'value').set;"
            " el.focus(); set.call(el, %s);"
            " el.dispatchEvent(new Event('input', {bubbles: true}));"
            " return 'typed'; })()" % (json.dumps(sel), json.dumps(text))
        )
        self.view.evaluate_javascript(js, -1, None, None, None, self.on_typed)

    def on_typed(self, view, result):
        try:
            verdict = view.evaluate_javascript_finish(result).to_string()
        except Exception as e:  # noqa: BLE001
            self.fail(f"could not type into {self.args.type!r}: {e}", 6)
            return
        if "missing" in verdict:
            self.fail(f"refusing: --type selector matched no element."
                      f" No screenshot written.", 5)
            return
        sel = self.args.type.split("::")[0]
        print(f"typed into {sel}", file=sys.stderr)
        if self.args.open:
            self.pending = list(self.args.open)
            GLib.timeout_add(int(self.args.settle * 1000),
                             lambda: (self.click_open(), False)[1])
            return
        GLib.timeout_add(int(self.args.settle * 1000), self.finish)

    def click_open(self):
        """Click the next element in the queue, then let it animate.

        A dropdown's contents do not exist in the DOM until it is opened, so a
        plain render of a page cannot photograph what a menu says - which is
        exactly where an empty menu tells a person they have no projects. The
        click is dispatched rather than synthesised at the pointer because the
        target may be off-screen in a wide harness page.

        `--open` may be given more than once, and then each selector is clicked
        in turn. One click reaches a menu; it does not reach what PRESSING
        something in that menu does, and a refusal only exists after the press.
        The power flyout is two clicks from the panel, so photographing its
        refused shutdown was impossible with a single one.
        """
        self.current = self.pending.pop(0)
        sel = json.dumps(self.current)
        self.view.evaluate_javascript(
            f"(() => {{ const el = document.querySelector({sel});"
            f" if (!el) return 'missing'; el.click(); return 'clicked'; }})()",
            -1, None, None, None, self.on_opened)

    def on_opened(self, view, result):
        try:
            verdict = view.evaluate_javascript_finish(result).to_string()
        except Exception as e:  # noqa: BLE001
            self.fail(f"could not click {self.args.open!r}: {e}", 6)
            return
        # A selector that matched nothing must refuse rather than quietly shoot
        # the unopened page: a screenshot of the thing not happening is the most
        # expensive kind of green.
        if "missing" in verdict:
            self.fail(f"refusing: --open {self.current!r} matched no element."
                      f" No screenshot written.", 5)
            return
        print(f"clicked {self.current}", file=sys.stderr)
        # Between clicks the wait is short: it only has to outlast the state
        # change that puts the next target in the DOM. The full settle is spent
        # once, before the shot, where an animation actually matters.
        if self.pending:
            GLib.timeout_add(300, lambda: (self.click_open(), False)[1])
            return
        after = self.finish
        GLib.timeout_add(int(self.args.settle * 1000), lambda: (after(), False)[1])

    def snapshot(self):
        self.view.get_snapshot(WebKit.SnapshotRegion.FULL_DOCUMENT,
                               WebKit.SnapshotOptions.NONE, None, self.on_snapshot)
        return False

    def on_snapshot(self, view, result):
        try:
            view.get_snapshot_finish(result).save_to_png(self.args.out)
        except Exception as e:  # noqa: BLE001
            self.fail(f"snapshot failed: {e}", 7)
            return
        print("wrote", self.args.out)
        # A frame of one flat colour is a shot of nothing, and it must not pass
        # silently. Measured on 24 August: a calendar whose backend was failing
        # came back 1920000 pixels of a single grey, which reads as "the app
        # renders nothing when its daemon is down" - a defect I nearly filed.
        # Rendered again by hand the same page showed its sidebar, its toolbar and
        # its refusal, so the black frame was this harness, not the app. A sweep
        # that records those quietly teaches the reader something false about
        # every app it touches.
        self.warn_if_flat()
        # A violation is a finding, not a rendering problem, so the PNG is still
        # written - but the exit status carries it, so this can be a gate.
        self.status = 1 if self.axe_failures else 0
        self.app.quit()

    def warn_if_flat(self):
        """Say so when the written frame carries no second colour."""
        try:
            from PIL import Image
        except ImportError:
            # Without it this check simply does not run. It must never be the
            # reason a shot fails.
            return
        try:
            with Image.open(self.args.out) as im:
                colours = im.convert("RGB").getcolors(maxcolors=2)
        except Exception:  # noqa: BLE001
            return
        # `getcolors` returns None once the image passes the cap, which is the
        # ordinary case and the one that needs no words.
        if colours is not None and len(colours) <= 1:
            print(
                "FLAT: every pixel in this frame is the same colour, so it shows "
                "nothing. Read it as a failure of this harness before reading it "
                "as a failure of the page."
            )


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--url", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--width", type=int, default=1280,
                    help="CSS viewport width to lay the page out at")
    ap.add_argument("--settle", type=float, default=1.5,
                    help="seconds after load before sizing")
    ap.add_argument("--reflow", type=float, default=1.0,
                    help="seconds after the zoom change, before measuring")
    ap.add_argument("--timeout", type=int, default=60,
                    help="give up if the page has not finished loading by then")
    ap.add_argument("--open", default=None, action="append",
                    help="CSS selector to click before the shot, for content that"
                         " only exists once a menu or panel is open. Repeatable:"
                         " each is clicked in turn, so a press INSIDE an opened"
                         " menu is reachable")
    ap.add_argument("--stub-host", action="store_true",
                    help="install a Tauri runtime whose every command FAILS, so "
                         "the page takes its backend-is-broken path rather than "
                         "its no-runtime path")
    ap.add_argument("--host-script", default=None,
                    help="path to a JS file installing the Tauri runtime this shot"
                         " needs - one that ANSWERS some commands and refuses"
                         " others, which is the state --stub-host cannot express."
                         " Replaces --stub-host when both are given")
    ap.add_argument("--probe", default=None,
                    help="evaluate one JS expression against the rendered page and"
                         " print its value; writes no image")
    ap.add_argument("--probe-file", default=None,
                    help="run a JS FILE (a body ending in `return`) against the"
                         " rendered page and print its value; the form the"
                         " dev/screenshot probes are written in, and the one that"
                         " works alongside --host-script")
    ap.add_argument("--axe", action="store_true",
                    help="run axe-core over the rendered page and print the"
                         " violations; exits non-zero if any are found")
    ap.add_argument("--type", default=None,
                    help="`selector::text` - put text in a field before the shot,"
                         " for copy that only appears once something is searched")
    ap.add_argument("--require-width", type=int, default=None,
                    help="refuse, before capturing, if the viewport is narrower")
    return Render(ap.parse_args()).run()


if __name__ == "__main__":
    sys.exit(main())
