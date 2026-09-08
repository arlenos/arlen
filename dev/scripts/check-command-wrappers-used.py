#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a frontend function which invokes a Tauri command has a caller.

`check-commands-invoked` counts a command as reached when its NAME appears
anywhere in the app's frontend. That is the right shape for what it catches and
it has one blind spot, found the day the notification history was built:
`notification_get_history` was named inside a store function nothing imported, so
the command passed the scan for weeks while the feature had no surface at all.
The daemon kept thirty days of notifications and an empty panel said "No
notifications" about a database with a month in it.

**A dead wrapper is invisible to a name scan**, because the name IS there. This
is the other half: the wrapper itself must be reached by something.

What counts as reached is deliberately generous - any mention of the name
anywhere else in the app, including its own file, including a template. A helper
called once by its own module is wired; the one this catches is mentioned
nowhere but where it is defined. That keeps every hit real, which matters more
here than catching every last one: a gate with false positives teaches people to
add exceptions.

The same `NO CALLER:` marker the command check honours works here, written above
the function, for a wrapper that is deliberately available rather than routinely
used.
"""

import re
import sys
from pathlib import Path

# Wrappers with no caller, by app, with why. Every one is a real finding rather
# than a tolerated shape: each wants the same answer as a dark command, which is
# call it or delete it.
CARRIED: dict[str, tuple[int, str]] = {
    "desktop-shell": (
        1,
        "`openWaypointer`: the launcher opens through the compositor's Super-tap "
        "protocol, which calls the Rust side directly, so this has never had a "
        "caller - and it would be wrong if it did, since it sets visible and then "
        "TOGGLES, closing a launcher that is already open",
    ),
    "terminal": (
        2,
        "`terminalGrid`, left from before the grid came through xterm, and "
        "`readCapability`, whose own doc says there is nowhere yet to put the "
        "sentence it returns",
    ),
}

# A function that is deliberately available rather than routinely used says so
# above itself, the same marker the command check reads.
SELF_EXCUSED = re.compile(r"NO CALLER:")

# `export function f`, `export async function f`, `export const f = (` and
# `export const f = async (`. An arrow assigned through a type annotation is
# covered by the optional `: T` before the `=`.
DEFINITION = re.compile(
    r"export\s+(?:async\s+)?function\s+(\w+)"
    r"|export\s+const\s+(\w+)\s*(?::[^=\n]+)?=\s*(?:async\s*)?\("
)

# Where one definition's body ends: the next top-level `export`. A fixed window
# was the first cut and it was wrong - 2000 characters ran past three short pure
# helpers into the next wrapper down the file, so the scan called
# `windowsOnWorkspace`, `minimizedCountFor` and `countFacets` command wrappers on
# the strength of an `invoke` that belongs to their neighbour. A gate that
# reports a function for what the one below it does is a gate people learn to
# ignore.
NEXT_DEFINITION = re.compile(r"\nexport ")

SUFFIXES = (".ts", ".svelte")
SKIP = ("/node_modules/", "/build/", "/.svelte-kit/", "/target/")


def sources(app_src: Path) -> dict[Path, str]:
    """Every frontend source of one app, by path."""
    out: dict[Path, str] = {}
    for path in app_src.rglob("*"):
        if path.suffix not in SUFFIXES or any(s in str(path) for s in SKIP):
            continue
        try:
            out[path] = path.read_text(errors="replace")
        except OSError:
            continue
    return out


def head_above(text: str, at: int) -> str:
    """The comment block immediately above a definition."""
    return text[:at].rsplit("\n\n", 1)[-1]


def unused_wrappers(texts: dict[Path, str]) -> list[str]:
    """The exported command wrappers nothing in this app mentions."""
    dark: list[str] = []
    for path, text in texts.items():
        if path.suffix != ".ts":
            continue
        for m in DEFINITION.finditer(text):
            name = m.group(1) or m.group(2)
            after = NEXT_DEFINITION.search(text, m.end())
            body = text[m.end() : after.start() if after else len(text)]
            if "invoke(" not in body and "invoke<" not in body:
                continue
            if SELF_EXCUSED.search(head_above(text, m.start())):
                continue
            uses = 0
            for other, other_text in texts.items():
                found = len(re.findall(r"\b" + re.escape(name) + r"\b", other_text))
                uses += found - 1 if other == path else found
            if uses == 0:
                dark.append(name)
    return sorted(dark)


def main() -> int:
    root = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
    apps_dir = root / "apps"
    if not apps_dir.is_dir():
        print("check-command-wrappers-used: no apps directory, so the scan is pointed wrong")
        return 1

    apps = 0
    problems: list[str] = []
    for app_dir in sorted(p for p in apps_dir.iterdir() if (p / "src").is_dir()):
        app = app_dir.name
        texts = sources(app_dir / "src")
        if not texts:
            continue
        apps += 1
        dark = unused_wrappers(texts)
        allowed, why = CARRIED.get(app, (0, ""))
        if len(dark) > allowed:
            problems.append(
                f"  - {app}: {len(dark)} wrapper(s) nothing calls, carried as {allowed}.\n"
                f"    {', '.join(dark)}"
            )
        elif len(dark) < allowed:
            problems.append(
                f"  - {app}: carried as {allowed} ({why}) and only {len(dark)} left. "
                "Lower the number so a new one cannot hide behind it."
            )

    if apps == 0:
        print("check-command-wrappers-used: no frontend sources found, so the scan is pointed wrong")
        return 1

    if problems:
        print("Command wrappers nothing calls:")
        print()
        print("\n".join(problems))
        print()
        print("  The command check sees the name and stops there. A wrapper nobody")
        print("  calls names it and reaches nobody. Call it, or delete it.")
        return 1

    carried = sum(n for n, _ in CARRIED.values())
    print(f"check-command-wrappers-used: {apps} app(s) scanned, {carried} carried")
    return 0


if __name__ == "__main__":
    sys.exit(main())
