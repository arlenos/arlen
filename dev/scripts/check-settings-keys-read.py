#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a config key the Settings app writes is one something reads.

A control that writes a key nobody reads is the quietest lie a settings page can
tell. The write succeeds, the file changes, the switch stays where it was put,
and the machine goes on doing what it did. Three of these were live on 9
September and two of them were privacy promises:

  * **`timeline.excluded_apps` and `timeline.excluded_paths`** - "Nothing these
    apps do is recorded", "Activity under these paths is never recorded". The
    knowledge daemon read one field of that section, `paused`, and recorded
    everything else. Somebody excluding their password manager was recorded.

  * **`focus_settings.default_suppressed_apps`** - "Whenever Focus Mode is
    active, notifications from these apps are silenced." The shell read the
    project's own `.project` file and nothing else, so there was no default to
    apply and the apps kept interrupting.

  * **`focus_settings.show_project_name`** - a switch that changed nothing.

WHAT IS CHECKED. Every `<store>.setValue("dotted.key", ...)` in the Settings
frontend. The store's name says which file the key lands in, and the table below
says which component owns that file; the key counts as read when its LAST
SEGMENT appears in that component's sources - as a struct field, a quoted
literal, or a TypeScript property. Generous on purpose: readers spell a key as a
serde field far more often than as a string, and a check that demanded the whole
dotted path would fail on every correctly-read key in the tree.

SCOPED TO THE OWNER, and that part is not generous, deliberately. An unscoped
search finds `end` and `mode` and `active` somewhere in a tree this size no
matter what, so a key nobody reads would pass on a coincidence - and passing on a
coincidence is the one failure this check cannot afford, because the thing it is
looking for is silence.

`compositor.toml` is skipped: its reader is the compositor, a separate repo. That
is a limit worth stating rather than papering over - seven keys are written into
it and nothing here can say whether they are read.
"""

import re
import sys
from pathlib import Path

# Which component owns each config file, by the Settings store that writes it.
# A file with no entry is skipped and named in the passing line, so a new store
# cannot quietly go unchecked.
OWNERS: dict[str, tuple[str, ...]] = {
    "shell": ("apps/desktop-shell",),
    "graph": ("daemons/knowledge",),
    "notifications": ("daemons/notification-daemon",),
    "ai": ("ai", "daemons/ai-engine-daemon", "daemons/config-broker"),
    "theme": ("sdk/theme", "apps/desktop-shell"),
}

# Written here, read in the compositor repo. Not checkable from this tree.
OUT_OF_TREE = ("compositor",)

# Keys written and knowingly unread, with why. Empty, and that is the point: the
# three that were here on 9 September are fixed rather than excused.
CARRIED: dict[str, str] = {}

WRITE = re.compile(r'\b([a-zA-Z_][\w]*)\.setValue\(\s*"([a-z][a-z0-9_]*(?:\.[a-z0-9_]+)+)"')

SOURCE_SUFFIXES = (".rs", ".ts", ".svelte")
SKIP = ("/node_modules/", "/target/", "/build/", "/.svelte-kit/")


def written_keys(settings_src: Path) -> dict[str, set[str]]:
    """Every dotted key the Settings frontend writes, by the store that writes it."""
    out: dict[str, set[str]] = {}
    for path in settings_src.rglob("*"):
        if path.suffix not in (".ts", ".svelte") or any(s in str(path) for s in SKIP):
            continue
        try:
            text = path.read_text(errors="replace")
        except OSError:
            continue
        for store, key in WRITE.findall(text):
            out.setdefault(store, set()).add(key)
    return out


def reads(roots: tuple[str, ...], root: Path, segment: str) -> bool:
    """Whether any source under `roots` names `segment` the way a reader would."""
    pattern = re.compile(
        rf'(pub {re.escape(segment)}\b|"{re.escape(segment)}"|\b{re.escape(segment)}\??:)'
    )
    for base in roots:
        directory = root / base
        if not directory.is_dir():
            continue
        for path in directory.rglob("*"):
            if path.suffix not in SOURCE_SUFFIXES or any(s in str(path) for s in SKIP):
                continue
            try:
                if pattern.search(path.read_text(errors="replace")):
                    return True
            except OSError:
                continue
    return False


def main() -> int:
    root = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
    settings_src = root / "apps" / "settings" / "src"
    if not settings_src.is_dir():
        print("check-settings-keys-read: no settings frontend, so the scan is pointed wrong")
        return 1

    by_store = written_keys(settings_src)
    if not by_store:
        print("check-settings-keys-read: no config writes found, so the scan is pointed wrong")
        return 1

    checked = 0
    skipped = 0
    unknown: list[str] = []
    problems: list[str] = []
    for store, keys in sorted(by_store.items()):
        if store in OUT_OF_TREE:
            skipped += len(keys)
            continue
        owners = OWNERS.get(store)
        if owners is None:
            unknown.append(store)
            continue
        for key in sorted(keys):
            if key in CARRIED:
                continue
            checked += 1
            if not reads(owners, root, key.split(".")[-1]):
                problems.append(
                    f"  - {key}: written to {store}.toml and nothing in "
                    f"{', '.join(owners)} reads it"
                )

    if unknown:
        problems.append(
            "  - no owner recorded for the "
            + ", ".join(sorted(unknown))
            + " store(s), so their keys were not checked. Add them to OWNERS or to "
            "OUT_OF_TREE with the reason."
        )

    if problems:
        print("Config keys the Settings app writes and nothing reads:")
        print()
        print("\n".join(problems))
        print()
        print("  A control that writes a key nobody reads leaves the switch where it")
        print("  was put and the machine doing what it did. Read it, or drop the control.")
        return 1

    print(
        f"check-settings-keys-read: {checked} key(s) written and read, "
        f"{skipped} skipped as compositor's (read in the other repo)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
