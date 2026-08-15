#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Check that no two apps claim the same dev port, and that each one agrees with itself.

WHY. On 16 August five apps collided: `greeter`, `system-monitor` and `viewers` all served
on 1429, `screenshot` and `terminal` both on 1425, and two more collisions were hidden one
level down - `desktop-shell`'s HMR port sat on `settings`' dev port, and `text-editor`'s sat
on `meetings`'. Every one of them also declared `strictPort: true`.

What that produces is the worst shape a failure can have. The second app's vite refuses to
start, its `tauri dev` window opens anyway, and it loads `devUrl` - which the FIRST app is
serving. So you get one app's interface inside another app's window, wired to that window's
commands, with nothing anywhere saying the two do not belong together. It looks like a
rendering bug in an app that is fine.

The rule this enforces:

  - every app's vite server port is unique across the tree;
  - every HMR port is unique, and never lands on any app's server port;
  - `devUrl` in `tauri.conf.json` names the app's OWN vite port, since a window that loads
    a port its own build does not serve is the same defect in one app.

HMR ports live in their own band (server + 100) rather than server + 1, because the +1
convention is what made two of the collisions: one app's HMR is the next app's server.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = (
    Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
)
APPS = ROOT / "apps"

PORT = re.compile(r"port: *(\d+)")
DEV_URL = re.compile(r'"devUrl": *"http://localhost:(\d+)"')


def ports_of(app: Path) -> tuple[int | None, int | None, int | None]:
    """(server, hmr, devUrl) for one app; any may be None."""
    vite = app / "vite.config.js"
    conf = app / "src-tauri/tauri.conf.json"
    server = hmr = dev = None
    if vite.is_file():
        found = [int(p) for p in PORT.findall(vite.read_text(encoding="utf-8", errors="replace"))]
        if found:
            server = found[0]
        if len(found) > 1:
            hmr = found[1]
    if conf.is_file():
        m = DEV_URL.search(conf.read_text(encoding="utf-8", errors="replace"))
        if m:
            dev = int(m.group(1))
    return server, hmr, dev


def main() -> int:
    if not APPS.is_dir():
        print("apps/ is missing; the layout moved and this check did not")
        return 1

    apps = {}
    for d in sorted(APPS.iterdir()):
        if not d.is_dir():
            continue
        server, hmr, dev = ports_of(d)
        if server is None and dev is None:
            continue
        apps[d.name] = (server, hmr, dev)

    if not apps:
        print("no app dev ports found; that is not plausible")
        return 1

    problems: list[str] = []

    # Every server port unique.
    servers: dict[int, list[str]] = {}
    for name, (server, _, _) in apps.items():
        if server is not None:
            servers.setdefault(server, []).append(name)
    for port, names in sorted(servers.items()):
        if len(names) > 1:
            problems.append(
                f"{', '.join(names)} all serve on {port}. With strictPort the second one's "
                f"vite refuses to start, its window opens anyway and loads whatever is on "
                f"that port - one app's interface inside another app's window"
            )

    # Every HMR port unique, and clear of every server port.
    hmrs: dict[int, list[str]] = {}
    for name, (_, hmr, _) in apps.items():
        if hmr is not None:
            hmrs.setdefault(hmr, []).append(name)
    for port, names in sorted(hmrs.items()):
        if len(names) > 1:
            problems.append(f"{', '.join(names)} share HMR port {port}")
        if port in servers:
            problems.append(
                f"{', '.join(names)} use {port} for HMR, which is {', '.join(servers[port])}'s "
                f"server port - the +1 convention is what made this happen, so HMR lives at "
                f"server + 100"
            )

    # Each app agrees with itself.
    for name, (server, _, dev) in sorted(apps.items()):
        if server is not None and dev is not None and server != dev:
            problems.append(
                f"{name} serves on {server} and its window loads {dev}: the window would "
                f"show whatever else is on {dev}, or nothing"
            )

    if problems:
        print("\napps disagree about dev ports:\n")
        for p in problems:
            print(f"  - {p}")
        return 1

    print(f"{len(apps)} app(s): every dev port, HMR port and devUrl distinct and self-consistent.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
