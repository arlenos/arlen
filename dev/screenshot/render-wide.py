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
import pathlib
import sys

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

    def on_activate(self, app):
        self.win = Gtk.ApplicationWindow(application=app)
        self.view = WebKit.WebView()
        self.view.set_hexpand(True)
        self.view.set_vexpand(True)
        self.win.set_child(self.view)
        # The only sizing call that this GTK honours - see the note at the top.
        self.win.fullscreen()
        self.win.present()
        self.view.connect("load-changed", self.on_load)
        self.view.load_uri(self.args.url)
        # A page that never finishes loading would otherwise hang the harness
        # forever, which in an unattended sweep looks like a machine that died.
        GLib.timeout_add_seconds(self.args.timeout, self.on_timeout)

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
        if self.args.axe and not self.args.type and not self.args.open:
            self.run_axe()
            return
        if self.args.type:
            self.type_into()
            return
        if self.args.open:
            self.pending = list(self.args.open)
            self.click_open()
            return
        self.snapshot()

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
        GLib.timeout_add(int(self.args.settle * 1000), self.snapshot)

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
        after = self.run_axe if self.args.axe else self.snapshot
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
        # A violation is a finding, not a rendering problem, so the PNG is still
        # written - but the exit status carries it, so this can be a gate.
        self.status = 1 if self.axe_failures else 0
        self.app.quit()


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
