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
import sys

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("WebKit", "6.0")
from gi.repository import GLib, Gtk, WebKit  # noqa: E402

# Below this the zoom needed to reach the requested width would shrink text past
# legibility in the PNG, and a picture nobody can read is not a verification.
MIN_ZOOM = 0.2
MAX_ZOOM = 6.0


class Render:
    """One window, one page, one measured capture."""

    def __init__(self, args):
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
        self.view.get_snapshot(WebKit.SnapshotRegion.FULL_DOCUMENT,
                               WebKit.SnapshotOptions.NONE, None, self.on_snapshot)

    def on_snapshot(self, view, result):
        try:
            view.get_snapshot_finish(result).save_to_png(self.args.out)
        except Exception as e:  # noqa: BLE001
            self.fail(f"snapshot failed: {e}", 7)
            return
        print("wrote", self.args.out)
        self.status = 0
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
    ap.add_argument("--require-width", type=int, default=None,
                    help="refuse, before capturing, if the viewport is narrower")
    return Render(ap.parse_args()).run()


if __name__ == "__main__":
    sys.exit(main())
