# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that an app grants itself every plugin command it invokes.

WHAT THIS IS FOR. Tauri denies a plugin command the app's capability file does not
name, and the denial arrives as a rejected promise. Thirteen apps called
`plugin:arlen-shell|menu_register` to publish their app menu into the top bar;
ONE had granted `arlen-shell:allow-menu-register`. Every one of those calls is
written `void invoke(...).catch(() => {})`, so ten app menus never appeared and
nothing anywhere said why - not a log, not a toast, not a test. Found on 27 August
by asking which permissions an app grants and never uses, and noticing the
question had an unasked twin.

The mapping is exact rather than guessed: `plugin:<plugin>|<command>` needs
`<plugin>:allow-<command-with-hyphens>`. No table to keep, no canonical-wrapper
assumption - the call names the permission it needs.

The reverse direction (a grant nobody calls) is `check-granted-and-used`, which
holds the two permissions with a single canonical ui-kit consumer. This is the
direction that fails at runtime and is swallowed, which is why it needs a check
rather than a boot.
"""

import json
import os
import pathlib
import re
import sys

ROOT = pathlib.Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else pathlib.Path(__file__).resolve().parents[2]
APPS = ROOT / "apps"
CALL = re.compile(r"plugin:([a-z][a-z0-9-]*)\|([a-z][a-z0-9_]*)")
SKIP = {"node_modules", ".svelte-kit", "target", "build", ".git"}

# App to the reason its calls are not held to this. MAY SHRINK, MAY NOT GROW.
UNRESOLVED: dict[str, str] = {
    "settings": (
        "calls `plugin:arlen-shell|menu_register` while loading a different plugin "
        "entirely (`tauri_plugin_arlen_menu`), so no grant would help: either the app "
        "moves to arlen-shell like its twelve siblings or the call moves to arlen-menu. "
        "Reported to the planner 27 Aug"
    ),
    "store": (
        "calls `plugin:arlen-shell|menu_register` with no arlen-shell grant and no "
        "plugin init found in its lib.rs, so it is not the one-line case the others "
        "were. Reported to the planner 27 Aug"
    ),
}


def permission_for(plugin: str, command: str) -> str:
    """The capability identifier a plugin command needs."""
    return f"{plugin}:allow-{command.replace('_', '-')}"


def calls_by_app(root: pathlib.Path) -> dict[str, set[tuple[str, str]]]:
    """Every `plugin:<name>|<command>` an app's frontend invokes."""
    out: dict[str, set[tuple[str, str]]] = {}
    if not root.is_dir():
        return out
    for app in sorted(p for p in root.iterdir() if p.is_dir()):
        src = app / "src"
        if not src.is_dir():
            continue
        for cur, dirs, files in os.walk(src):
            dirs[:] = sorted(d for d in dirs if d not in SKIP)
            for name in sorted(files):
                if not name.endswith((".ts", ".svelte", ".js")):
                    continue
                text = (pathlib.Path(cur) / name).read_text(errors="replace")
                for plugin, command in CALL.findall(text):
                    out.setdefault(app.name, set()).add((plugin, command))
    return out


def grants(app_dir: pathlib.Path) -> set[str]:
    """Every permission identifier the app's capability files name."""
    out: set[str] = set()
    caps = app_dir / "src-tauri/capabilities"
    if not caps.is_dir():
        return out
    for path in sorted(caps.glob("*.json")):
        try:
            doc = json.loads(path.read_text(errors="replace"))
        except json.JSONDecodeError:
            continue
        for entry in doc.get("permissions", []):
            if isinstance(entry, str):
                out.add(entry)
            elif isinstance(entry, dict) and isinstance(entry.get("identifier"), str):
                out.add(entry["identifier"])
    return out


def main() -> int:
    if not APPS.is_dir():
        print(f"NOTHING WAS READ: no apps directory at {APPS}", file=sys.stderr)
        return 2
    calls = calls_by_app(APPS)
    if not calls:
        print(
            "NOTHING WAS READ: no app invokes a plugin command, so no grant was "
            "compared. The call spelling changed or this is not the tree it thinks",
            file=sys.stderr,
        )
        return 2

    findings: list[str] = []
    checked = 0
    excused = 0
    for app, pairs in sorted(calls.items()):
        if app in UNRESOLVED:
            excused += 1
            continue
        held = grants(APPS / app)
        for plugin, command in sorted(pairs):
            checked += 1
            needed = permission_for(plugin, command)
            if needed not in held:
                findings.append(
                    f"{app} invokes `plugin:{plugin}|{command}` and its capability "
                    f"file does not grant `{needed}`. Tauri rejects the call, and a "
                    f"rejected invoke is a promise nobody is watching"
                )

    # Stale only when the app is THERE and calls nothing. An excuse for an app
    # this tree does not contain is not stale, it is out of scope - and reporting
    # it made every small fixture fail for a reason that had nothing to do with
    # the case under test.
    stale = sorted(
        a for a in UNRESOLVED if (APPS / a).is_dir() and a not in calls
    )
    for a in stale:
        findings.append(f"{a} is excused here and invokes no plugin command; remove the entry")

    if findings:
        print(f"{checked} plugin call(s) checked, {len(findings)} finding(s):\n", file=sys.stderr)
        for f in findings:
            print(f"  - {f}", file=sys.stderr)
        return 1

    print(
        f"{checked} plugin command call(s) across {len(calls) - excused} app(s), each "
        f"granted by the app that makes it; {excused} app(s) excused with a reason. A "
        f"denied plugin call rejects a promise, and these are all written to discard it."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
