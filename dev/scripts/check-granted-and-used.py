# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that an app which grants the shell theme permission actually reads it.

A Tauri capability file is a grant, and a grant nobody exercises is two problems
wearing one coat. It is a permission the app holds for no reason - the smaller
half - and it is a strong hint that a feature was wired everywhere except here,
which is the half that shows.

That is what it was: on 9 August, three apps (clock, knowledge and the text
editor) declared `arlen-shell:allow-theme-get` and never called it. They sat in
the default palette while the other six followed the system theme, and nothing
said so, because a missing call is invisible from both sides - the capability
file looks complete and the layout looks like every other layout, minus one line.

What this compares, per app under `apps/`:

  * does its capability file grant `arlen-shell:allow-theme-get`
  * does anything under its `src/` call `initArlenTheme` (the ui-kit consumer,
    which is what actually invokes `plugin:arlen-shell|theme_get`)

and reports a grant with no call. The reverse - a call with no grant - would fail
loudly at runtime the first time the app started, so it is not what needs a
check; this is the direction that fails silently and looks fine.

What it does NOT cover:

  * any other permission. `theme_get` is checkable because there is exactly one
    canonical consumer; `locale_get` has the same shape and could be added, and
    the window permissions are used from markup rather than a named call.
  * whether the theme is APPLIED correctly once read. The kit owns that, and a
    screenshot is what checks it.
  * `apps/harness` and `apps/store`, arlen-ui's live work.

Shown to fail before being trusted: removing the `initArlenTheme` line from any
wired app's layout makes it name that app.
"""

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

GRANT = "arlen-shell:allow-theme-get"
CONSUMER = "initArlenTheme"
SKIP = {"harness", "store"}

# An app that grants the permission and does not read it for a reason. Empty is
# the goal: a grant nobody uses should be dropped or used, not explained.
ACKNOWLEDGED: dict[str, str] = {}


def grants_theme(app: Path) -> bool:
    caps = app / "src-tauri" / "capabilities"
    if not caps.is_dir():
        return False
    for f in caps.glob("*.json"):
        try:
            data = json.loads(f.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue
        for perm in data.get("permissions", []):
            name = perm if isinstance(perm, str) else perm.get("identifier")
            if name == GRANT:
                return True
    return False


def reads_theme(app: Path) -> bool:
    src = app / "src"
    if not src.is_dir():
        return False
    for f in list(src.rglob("*.svelte")) + list(src.rglob("*.ts")):
        if CONSUMER in f.read_text(encoding="utf-8", errors="replace"):
            return True
    return False


def main() -> int:
    findings: list[str] = []
    granted = 0
    for app in sorted(p for p in (ROOT / "apps").iterdir() if p.is_dir()):
        if app.name in SKIP or not grants_theme(app):
            continue
        granted += 1
        if reads_theme(app) or app.name in ACKNOWLEDGED:
            continue
        findings.append(
            f"apps/{app.name}: grants `{GRANT}` and never calls `{CONSUMER}`, so it "
            f"holds a permission it does not use and sits in the default palette "
            f"while the rest of the desktop follows the system theme"
        )

    print(
        f"{granted} app(s) grant the shell theme permission; each checked for a "
        f"reader. One permission only - it is the one with a single canonical "
        f"consumer to look for."
    )
    if findings:
        print("\ngranted and never read:\n")
        for f in findings:
            print(f"  - {f}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
