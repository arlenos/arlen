#!/usr/bin/env python3
"""An app has one name, in each language, wherever it is written down.

A name lives in three places and they are read by different things:

  * `dist/*.desktop` `Name` / `Name[de]` - what the launcher lists it as.
  * the catalog's `*.app.title` - what the window is named at runtime, since
    every app now sets its native title from it.
  * `tauri.conf.json` `title` - the name the window carries before the webview
    has booted, and the one it keeps if the app has no catalog.

They drifted, and a German boot showed how that reads: the system monitor is
`Systemmonitor` in the launcher, `Task-Manager` in its own title bar and
`System Monitor` in its config, so one app answers to three names on one
machine. Nothing about that is catchable by a test of any single file, which is
why it survived until somebody looked at the screen.

What this refuses: a `.desktop` name that disagrees with the catalog, per
locale, and a `tauri.conf.json` title that disagrees with the catalog's source
language. An app with no catalog is not judged - there is nothing to disagree
with.
"""

import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]

# A name that two places genuinely disagree about, where deciding is somebody
# else's call rather than a typo to fix.
ACKNOWLEDGED = {
    "system-monitor": (
        "the launcher says Systemmonitor, the catalog says Task-Manager and the "
        "config says System Monitor. Which of the three the app is called is a "
        "product decision, not a drift to silently pick a winner for"
    ),
}


def catalog_title(app: pathlib.Path):
    """The app's own name in the source language and in German, or None."""
    cat = app / "src/lib/i18n/messages.ts"
    if not cat.exists():
        return None
    found = re.findall(r'"([a-z.]*\.app\.title)":\s*"([^"]*)"', cat.read_text(encoding="utf-8"))
    if len(found) < 2:
        return None
    return found[0][1], found[1][1]


def desktop_names(app: pathlib.Path):
    """`Name` and `Name[de]` from the app's desktop entry, or None."""
    entries = sorted((app / "dist").glob("*.desktop")) if (app / "dist").exists() else []
    if not entries:
        return None
    text = entries[0].read_text(encoding="utf-8")

    def field(key):
        m = re.search(rf"^{re.escape(key)}=(.*)$", text, re.M)
        return m.group(1).strip() if m else None

    return entries[0], field("Name"), field("Name[de]")


def conf_title(app: pathlib.Path):
    conf = app / "src-tauri/tauri.conf.json"
    if not conf.exists():
        return None
    try:
        d = json.loads(conf.read_text(encoding="utf-8"))
    except json.JSONDecodeError:
        return None
    windows = d.get("app", {}).get("windows") or []
    return windows[0].get("title") if windows else None


def main() -> int:
    findings = []
    checked = 0
    for app in sorted((ROOT / "apps").iterdir()):
        if not app.is_dir():
            continue
        titles = catalog_title(app)
        if titles is None:
            continue
        desk = desktop_names(app)
        if desk is None:
            # No desktop entry means nobody launches it and no bar shows its
            # name: the shell is a layer surface and the greeter owns the whole
            # screen, so their window titles are strings no person ever reads.
            # Judging those produced two findings that were not defects.
            continue
        en, de = titles
        checked += 1
        if app.name in ACKNOWLEDGED:
            continue

        if desk:
            entry, d_en, d_de = desk
            rel = entry.relative_to(ROOT)
            if d_en is not None and d_en != en:
                findings.append(f"{rel}: launcher says {d_en!r}, the app calls itself {en!r}")
            if d_de is not None and d_de != de:
                findings.append(f"{rel}: launcher says {d_de!r} in German, the app calls itself {de!r}")

        title = conf_title(app)
        if title is not None and title != en:
            rel = (app / "src-tauri/tauri.conf.json").relative_to(ROOT)
            findings.append(f"{rel}: window starts as {title!r}, the app calls itself {en!r}")

    for f in findings:
        print(f"  - {f}")
    if findings:
        print()
        print(
            "One app, one name per language. The launcher, the window and the\n"
            "config are read by different people at different moments, and a\n"
            "disagreement between them is a machine that answers to two names."
        )
        return 1
    print(
        f"check-app-names-agree: {checked} app(s) name themselves in a catalog, "
        f"{len(ACKNOWLEDGED)} acknowledged, the rest agree everywhere they are written."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
