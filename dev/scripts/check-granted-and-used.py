# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that an app which grants a shell permission actually uses it.

A Tauri capability file is a grant, and a grant nobody exercises is two problems
wearing one coat. It is a permission the app holds for no reason - the smaller
half - and it is a strong hint that a feature was wired everywhere except here,
which is the half that shows.

That is what it was: on 9 August, three apps (clock, knowledge and the text
editor) declared `arlen-shell:allow-theme-get` and never called it. They sat in
the default palette while the other six followed the system theme, and nothing
said so, because a missing call is invisible from both sides - the capability
file looks complete and the layout looks like every other layout, minus one line.

What this compares, per app under `apps/`, for each permission in the table
below: does the capability file grant it, and does anything under `src/` call the
ui-kit function that invokes it. A grant with no call is reported. The reverse -
a call with no grant - fails loudly at runtime the first time the app starts, so
it is not what needs a check; this is the direction that fails silently and looks
fine.

`locale_get` joined `theme_get` on the same day, after the three theme fixes: it
was already consistent across all nine apps, and locking in a property while it
holds is cheaper than rediscovering it broken.

What it does NOT cover:

  * permissions with no single named consumer. The window permissions are used
    from markup rather than a call, so there is nothing to look for.
  * whether the theme is APPLIED correctly once read. The kit owns that, and a
    screenshot is what checks it.
  * `apps/harness` and `apps/store`, arlen-ui's live work.

Shown to fail before being trusted: removing the `initArlenTheme()` call from any
wired app's layout makes it name that app - and the first attempt at that proof
is what found the import-versus-call bug in this file.
"""

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# Permission to the ui-kit call that exercises it. Only permissions with exactly
# one canonical consumer belong here; anything else would need a guess.
PAIRS = {
    "arlen-shell:allow-theme-get": "initArlenTheme",
    "arlen-shell:allow-locale-get": "initArlenLocale",
}
SKIP = {"harness", "store"}

# An app that grants the permission and does not read it for a reason. Empty is
# the goal: a grant nobody uses should be dropped or used, not explained.
ACKNOWLEDGED: dict[str, str] = {}


def granted(app: Path) -> set[str]:
    """The permission identifiers this app's capability files declare."""
    out: set[str] = set()
    caps = app / "src-tauri" / "capabilities"
    if not caps.is_dir():
        return out
    for f in caps.glob("*.json"):
        try:
            data = json.loads(f.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue
        for perm in data.get("permissions", []):
            name = perm if isinstance(perm, str) else perm.get("identifier")
            if name:
                out.add(name)
    return out


def calls(app: Path, consumer: str) -> bool:
    """Whether the app CALLS the consumer, not merely imports it.

    The difference is not pedantry: the first version of this check looked for the
    bare name, so deleting the call from a layout left the import behind and the
    check stayed green on an app that had just lost the feature. Found by trying
    to make it fail.
    """
    src = app / "src"
    if not src.is_dir():
        return False
    needle = f"{consumer}("
    for f in list(src.rglob("*.svelte")) + list(src.rglob("*.ts")):
        text = f.read_text(encoding="utf-8", errors="replace")
        for line in text.splitlines():
            stripped = line.strip()
            if stripped.startswith("import ") or stripped.startswith("//"):
                continue
            if needle in stripped:
                return True
    return False


def main() -> int:
    findings: list[str] = []
    pairs_checked = 0
    for app in sorted(p for p in (ROOT / "apps").iterdir() if p.is_dir()):
        if app.name in SKIP:
            continue
        have = granted(app)
        for permission, consumer in PAIRS.items():
            if permission not in have:
                continue
            pairs_checked += 1
            if calls(app, consumer) or f"{app.name}:{permission}" in ACKNOWLEDGED:
                continue
            findings.append(
                f"apps/{app.name}: grants `{permission}` and never calls "
                f"`{consumer}`, so it holds a permission it does not use - and "
                f"whatever that permission was for is not happening in this app"
            )

    print(
        f"{pairs_checked} granted permission(s) checked for the call that uses "
        f"them, across {len(PAIRS)} permission(s) with a single canonical consumer."
    )
    if findings:
        print("\ngranted and never read:\n")
        for f in findings:
            print(f"  - {f}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
