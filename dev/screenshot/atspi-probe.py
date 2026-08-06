#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Walk the AT-SPI tree of a running app and print what it exposes.

The boot gate has been wrong in both directions on the same consent dialog,
because a screenshot only answers "did these pixels change". AT-SPI answers the
question the gate actually has: is there a button named "Allow once", does it
have focus, is it still on screen. That is a fact about the widget, not about a
frame.

The open question this exists to settle is how far it reaches into a Tauri app.
The window, its dialogs and any native widget are the safe expectation; the
WebKitGTK webview's INTERIOR is not, because the plumbing that grafts a
webview's DOM into the host accessibility tree is the GTK4 path and Tauri here
links the GTK3 line. So this prints the tree with depth and role, and says
plainly whether anything below the webview showed up.

Run it against an app you have already started on the same accessibility bus:

    dev/screenshot/atspi-probe.py                 # every app on the bus
    dev/screenshot/atspi-probe.py --app settings  # one, by name substring
"""

import argparse
import sys

import pyatspi

# Roles that mean "this is the webview container"; anything below one of these
# is DOM rather than native widgetry, which is the thing worth knowing.
WEB_ROLES = {"document web", "document frame", "embedded", "html container"}

MAX_CHILDREN = 40


def describe(node):
    try:
        name = node.name or ""
    except Exception:
        name = "<unreadable>"
    try:
        role = node.getRoleName()
    except Exception:
        role = "<unreadable>"
    try:
        states = node.getState().getStates()
        flags = [pyatspi.stateToString(s) for s in states]
    except Exception:
        flags = []
    interesting = [f for f in flags if f in ("focused", "showing", "visible", "sensitive")]
    return role, name, interesting


def walk(node, depth, out, seen_web):
    role, name, flags = describe(node)
    if role in WEB_ROLES:
        seen_web.append(depth)
    label = f"{'  ' * depth}{role}"
    if name:
        label += f'  "{name[:60]}"'
    if flags:
        label += f"  [{', '.join(flags)}]"
    out.append(label)

    try:
        count = node.childCount
    except Exception:
        return
    # A deep tree is normal; the probe is about what KINDS of node appear, so cap
    # the breadth rather than printing a whole document.
    for i in range(min(count, MAX_CHILDREN)):
        try:
            child = node.getChildAtIndex(i)
        except Exception:
            continue
        if child is not None:
            walk(child, depth + 1, out, seen_web)
    if count > MAX_CHILDREN:
        out.append(f"{'  ' * (depth + 1)}... {count - MAX_CHILDREN} more sibling(s)")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--app", default=None, help="only apps whose name contains this")
    ap.add_argument("--depth", type=int, default=12, help="how deep to walk")
    args = ap.parse_args()

    desktop = pyatspi.Registry.getDesktop(0)
    apps = []
    for i in range(desktop.childCount):
        app = desktop.getChildAtIndex(i)
        if app is None:
            continue
        if args.app and args.app.lower() not in (app.name or "").lower():
            continue
        apps.append(app)

    if not apps:
        which = f" matching {args.app!r}" if args.app else ""
        print(f"no application{which} on the accessibility bus", file=sys.stderr)
        # Not an error: an empty bus is a real answer, and the caller decides.
        return 1

    for app in apps:
        out, seen_web = [], []
        walk(app, 0, out, seen_web)
        print("\n".join(out[: args.depth * 200]))
        print()
        if seen_web:
            print(f"  -> a web document node appears at depth {seen_web[0]}; "
                  f"{len(out)} node(s) total, so the DOM IS reachable")
        else:
            print("  -> no web-document node: native widgetry only, the webview "
                  "interior is opaque from here")
        print()
    return 0


if __name__ == "__main__":
    sys.exit(main())
