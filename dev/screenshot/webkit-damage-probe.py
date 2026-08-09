#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Does WebKit report damage for the region a shrinking element vacates?

PR-20's ghost was filed against the compositor and the compositor is not at
fault: `ghost-repro` now measures five shapes, and the only one that leaves a
stale frame is the one where the CLIENT damages less than it repainted. The
waypointer's layer surface is anchored to all four edges, so it never resizes -
only the card drawn inside it shrinks, and the strip that card vacates is inside
the surface, which makes reporting damage for it the client's job. The client
there is WebKit.

This asks that question with nothing else in the frame: no Tauri, no shell, no
layer shell, no input. A GTK window with one WebView, a page that paints a large
magenta block and then shrinks it on its own timer. Whatever occupies the region
the block gave up is the answer - the page background if WebKit damaged it, the
block's own magenta if it did not.

The colour is the same saturated magenta `ghost-repro` uses, for the same reason:
it appears in no theme, so a pixel readback attributes it without argument.

**A plain toplevel is faithful here even though the waypointer is a layer
surface.** What is being asked is whether a shrinking element inside an
unchanging surface leaves stale pixels, and the surface's role does not enter
into that. It also sidesteps two dead ends measured today: gtk4-layer-shell has
no Python typelib on this host (GtkLayerShell 0.1 is GTK 3, WebKit 6.0 is GTK 4),
and `shoot-compositor.sh` records that injecting input into a nested surface
under Xvfb is unsolved - so a probe that needed either would not run at all.

Usage, under the nested compositor:

  dev/screenshot/shoot-compositor.sh /tmp/webkit-damage.png \\
    dev/screenshot/webkit-damage-probe.py 1200

The argument is how long to hold the big block before shrinking it, in ms. It
must be shorter than the harness's settle, or the capture lands on the block and
the shrink never gets photographed - the same trap `ghost-repro` documents.
"""
import sys

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("WebKit", "6.0")
from gi.repository import GLib, Gtk, WebKit  # noqa: E402

# Magenta on a dark ground, and the block is deliberately not centred: a stale
# region has to be attributable to the block's OLD bounds rather than to
# "something magenta is on screen", so the two sizes share a top-left corner.
PAGE = """<!doctype html>
<html><body style="margin:0;background:#262829">
  <div id="b" style="position:absolute;left:120px;top:120px;
       width:520px;height:520px;background:#ff00ff"></div>
</body></html>"""

SHRINK = "document.getElementById('b').style.width='140px';" \
         "document.getElementById('b').style.height='140px';"


def main():
    hold_ms = int(sys.argv[1]) if len(sys.argv) > 1 else 1200
    app = Gtk.Application(application_id="dev.arlen.webkit-damage-probe")

    def activate(a):
        win = Gtk.ApplicationWindow(application=a)
        win.set_default_size(900, 900)
        view = WebKit.WebView()
        view.set_hexpand(True)
        view.set_vexpand(True)
        win.set_child(view)
        win.present()
        view.load_html(PAGE, None)

        def report(v, result):
            # The numbers, because the first run of this could not be read: the
            # window came up floated and small, the page laid out at a scale I had
            # not measured, and "is that magenta the small block or the big one
            # clipped" was a judgement call about a screenshot. It is not one any
            # more - the block says where it is and how big, in device pixels, and
            # the capture either matches that or it does not.
            try:
                print("after shrink:", v.evaluate_javascript_finish(result).to_string(),
                      file=sys.stderr)
            except Exception as e:  # noqa: BLE001
                print(f"could not read the block back: {e}", file=sys.stderr)

        def shrink():
            # Through the page rather than by resizing the window: resizing would
            # change the surface, and a surface change is the compositor's to
            # repair - which is the thing already measured clean. The question
            # here is strictly about what WebKit does inside a surface that stays
            # the size it was.
            view.evaluate_javascript(SHRINK, -1, None, None, None, None)
            GLib.timeout_add(300, measure)
            return False

        def measure():
            view.evaluate_javascript(
                "(() => { const r = document.getElementById('b').getBoundingClientRect();"
                " const d = window.devicePixelRatio;"
                " return JSON.stringify({dpr: d, css: [r.left, r.top, r.width, r.height],"
                " device: [r.left*d, r.top*d, r.width*d, r.height*d],"
                " viewport: [innerWidth*d, innerHeight*d]}); })()",
                -1, None, None, None, report)
            return False

        GLib.timeout_add(hold_ms, shrink)

    app.connect("activate", activate)
    # Long enough that the capture lands after the shrink and the surface is still
    # alive: dropping the connection would take the evidence with it.
    GLib.timeout_add_seconds(120, lambda: app.quit() or False)
    return app.run([])


if __name__ == "__main__":
    sys.exit(main())
