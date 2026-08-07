# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that an app invokes commands ITS OWN binary defines.

`check-invoke-shape` asks whether a command exists anywhere in the tree, and
says so plainly: a call into a command defined elsewhere "reads as a scoping
question rather than a dead call". This is that question, asked.

It matters because a Tauri command is not a service. It is a function compiled
into one app's binary and registered on that app's handler, so an app whose
webview invokes a command another app defines gets a rejected call - the same
failure as invoking a name nobody defines, hidden by the fact that `grep` finds
the name. Commands from a shared PLUGIN are different: the plugin registers them
into every app that loads it, so they are resolved per app here rather than
counted as the defining app's.

**Not in `just checks`, deliberately**, and for the same reason
`check-dbus-callers` is not: it currently reports real, pre-existing calls, and
declaring them to reach a green would be the wrong way round. Run it by hand;
wire it in when the list is decided.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
SKIP_PARTS = {"target", "node_modules", ".git", "build", ".svelte-kit"}

COMMAND = re.compile(r"#\[tauri::command[^\]]*\]\s*(?:pub\s+)?(?:async\s+)?fn\s+(\w+)")
INVOKE = re.compile(r'invoke(?:<[^>]*>)?\(\s*"([a-z_][a-z0-9_]*)"')


def app_of(path: pathlib.Path) -> str | None:
    """The app a file belongs to, or `None` for anything outside `apps/`."""
    parts = path.relative_to(ROOT).parts
    return parts[1] if len(parts) > 2 and parts[0] == "apps" else None


def scan(pattern: re.Pattern[str], suffixes: tuple[str, ...]) -> dict[str, set[str]]:
    """Every match of `pattern`, grouped by the app the file belongs to."""
    out: dict[str, set[str]] = {}
    for path in ROOT.rglob("*"):
        if path.suffix not in suffixes or SKIP_PARTS & set(path.parts):
            continue
        app = app_of(path)
        if app is None:
            continue
        try:
            body = path.read_text()
        except (UnicodeDecodeError, OSError):
            continue
        for m in pattern.finditer(body):
            out.setdefault(app, set()).add(m.group(1))
    return out


def plugin_commands() -> set[str]:
    """Commands a shared SDK plugin registers, which every app that loads it
    gets. Read from the plugin crates rather than assumed, so a plugin gaining a
    command does not turn into a false finding here."""
    out = set()
    for path in (ROOT / "sdk").rglob("*.rs"):
        if SKIP_PARTS & set(path.parts):
            continue
        try:
            body = path.read_text()
        except (UnicodeDecodeError, OSError):
            continue
        out.update(m.group(1) for m in COMMAND.finditer(body))
    return out


def main() -> int:
    defines = scan(COMMAND, (".rs",))
    invokes = scan(INVOKE, (".ts", ".svelte", ".js"))
    shared = plugin_commands()
    if not defines:
        print("found no #[tauri::command] functions under apps/; the check needs updating")
        return 2

    problems = []
    for app in sorted(invokes):
        own = defines.get(app, set())
        for cmd in sorted(invokes[app]):
            if cmd in own or cmd in shared:
                continue
            elsewhere = sorted(a for a, cs in defines.items() if cmd in cs)
            if not elsewhere:
                continue  # nobody defines it: check-invoke-shape's finding, not this one
            problems.append(
                f"{app} invokes `{cmd}`, which only {', '.join(elsewhere)} define(s). "
                f"A command lives in one app's binary, so this call is rejected at "
                f"runtime - move it into {app}, or into a shared plugin every app loads"
            )

    if problems:
        print("calls into another app's command:\n")
        for p in problems:
            print(f"  - {p}")
        return 1

    total = sum(len(v) for v in invokes.values())
    print(
        f"{len(invokes)} app(s), {total} distinct invoke(s), "
        f"{len(shared)} shared plugin command(s): every call resolves in its own app"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
