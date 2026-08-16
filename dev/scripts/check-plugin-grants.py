#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Check that an app is granted every shell-plugin command its frontend calls.

WHY. On 16 August the Files app pushed its breadcrumb into the topbar on every navigation -
its own module header quotes the plan for it - while `capabilities/default.json` granted only
`allow-theme-get` and `allow-locale-get`. Every navigation raised

    arlen-shell.toolbar_set_breadcrumb not allowed.

and the topbar never showed where you were. The terminal had the identical defect with
`toolbar.setQuickActions`. Two of the two apps that call the plugin directly were both wrong.

This failure hides better than most. It compiles, the tests pass, the screenshot looks right
(the feature is simply absent), and the call site is `void`-ed so nothing downstream breaks -
the only evidence is a rejected promise in a console nobody is reading. It is exactly the
"wired in code, inert in deployment" shape the profile and unit checks already guard on the
daemon side; this is the same rule for the webview side.

HOW. `sdk/tauri-plugin-shell/index.ts` is the single source for the mapping: each exported
object holds methods whose bodies invoke `plugin:arlen-shell|<command>`. This reads that file
to learn `(object, method) -> command`, then for every app that IMPORTS those objects finds
the calls it makes and requires `arlen-shell:allow-<command-with-dashes>` in its capability
file.

SCOPE, deliberately: only an app's OWN source is scanned. A call made on the app's behalf by
shared kit code (the theme consumer, say) still needs its grant, but resolving that needs a
module graph rather than a regex - and the apps carrying those grants today already have them.
This checks the direction that crashes: called here, not granted here. Over-granting is not
failed, because a permission is sometimes granted for a kit-mediated call this cannot see.
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
PLUGIN_API = ROOT / "sdk/tauri-plugin-shell/index.ts"

PLUGIN_IMPORT = re.compile(r'import\s*\{([^}]*)\}\s*from\s*"@arlen/tauri-plugin-shell"')
EXPORT_OBJ = re.compile(r"^export const (\w+) = \{", re.M)
METHOD = re.compile(r"^\s{2}(?:async )?(\w+)\s*[(<]", re.M)
COMMAND = re.compile(r"\$\{PLUGIN\}\|(\w+)")


def command_map(api: str) -> dict[tuple[str, str], str]:
    """`(exported object, method) -> plugin command`, read from the API module."""
    out: dict[tuple[str, str], str] = {}
    starts = [(m.group(1), m.start()) for m in EXPORT_OBJ.finditer(api)]
    for i, (obj, start) in enumerate(starts):
        end = starts[i + 1][1] if i + 1 < len(starts) else len(api)
        block = api[start:end]
        # Walk the block method by method so a command is attributed to the method
        # whose body contains it, not to the nearest one textually.
        methods = [(m.group(1), m.start()) for m in METHOD.finditer(block)]
        for j, (name, mstart) in enumerate(methods):
            mend = methods[j + 1][1] if j + 1 < len(methods) else len(block)
            cmd = COMMAND.search(block[mstart:mend])
            if cmd:
                out[(obj, name)] = cmd.group(1)
    return out


def permission(command: str) -> str:
    return "arlen-shell:allow-" + command.replace("_", "-")


def app_sources(app: Path) -> list[Path]:
    src = app / "src"
    if not src.is_dir():
        return []
    return [
        p
        for p in src.rglob("*")
        if p.suffix in {".ts", ".svelte", ".js"} and "node_modules" not in p.parts
    ]


def main() -> int:
    if not PLUGIN_API.is_file():
        print(f"{PLUGIN_API.relative_to(ROOT)} is missing; the plugin API moved and this did not")
        return 1

    commands = command_map(PLUGIN_API.read_text(encoding="utf-8", errors="replace"))
    if not commands:
        print("no plugin commands parsed out of the API module; that is not plausible")
        return 1

    problems: list[str] = []
    checked = 0

    for app in sorted(p for p in APPS.iterdir() if p.is_dir()):
        cap = app / "src-tauri/capabilities/default.json"
        if not cap.is_file():
            continue
        try:
            granted = set(json.loads(cap.read_text(encoding="utf-8")).get("permissions", []))
        except json.JSONDecodeError as e:
            problems.append(f"{app.name}: capabilities/default.json does not parse: {e}")
            continue

        for path in app_sources(app):
            text = path.read_text(encoding="utf-8", errors="replace")
            imported: set[str] = set()
            for m in PLUGIN_IMPORT.finditer(text):
                for piece in m.group(1).split(","):
                    name = piece.split(" as ")[-1].strip()
                    if name and not name.startswith("type"):
                        imported.add(name)
            if not imported:
                continue
            checked += 1
            for (obj, method), command in commands.items():
                if obj not in imported:
                    continue
                if not re.search(rf"\b{re.escape(obj)}\.{re.escape(method)}\s*\(", text):
                    continue
                need = permission(command)
                if need not in granted:
                    problems.append(
                        f"{app.name}: {path.relative_to(ROOT)} calls {obj}.{method}() "
                        f"({command}) without {need} - the call is refused at runtime and the "
                        f"feature is silently absent"
                    )

    if problems:
        print("\napps call shell-plugin commands they were never granted:\n")
        for p in sorted(set(problems)):
            print(f"  - {p}")
        print(
            "\nAdd the permission to that app's capabilities/default.json, or drop the call. "
            "Granting one it does not call is the other half of the same mistake."
        )
        return 1

    print(f"{checked} frontend file(s) calling the shell plugin: every command is granted.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
