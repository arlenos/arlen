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

**Not in `just checks`, deliberately**: it currently reports real, pre-existing
calls, and declaring them to reach a green would be the wrong way round. Run it by
hand; wire it in when the list is decided. (`check-dbus-callers` was in the same
state and is now green and in CI, so this is the last one waiting.)

The two it reports are the same shape and neither is fixed by moving code: the
command lives in `desktop-shell` because the shell owns the thing - the global
menu bar, the top bar's inventory - and the caller is another app that needs to
ask the shell for it. A Tauri command cannot cross a binary, so what those two
need is an IPC path, which is a decision about mechanism rather than a repair.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
SKIP_PARTS = {"target", "node_modules", ".git", "build", ".svelte-kit"}

COMMAND = re.compile(r"#\[tauri::command[^\]]*\]\s*(?:pub\s+)?(?:async\s+)?fn\s+(\w+)")
INVOKE = re.compile(r'invoke(?:<[^>]*>)?\(\s*"([a-z_][a-z0-9_]*)"')
# A call made through a local helper - `send(cmd, args)` wrapping `invoke(cmd,
# args)` - carries no literal for the pattern above to find, so it is invisible
# here exactly as it was in `check-invoke-shape`, where the clock's fifteen
# actions were being counted past. This check asks a different question of the
# same calls (whose binary defines them), so the blind spot means a wrapped call
# into ANOTHER app's command would not be reported at all. Named in the summary
# rather than resolved: following a wrapper can be confidently wrong.
WRAPPED = re.compile(r'\binvoke\s*(?:<[^>]*>)?\s*\(\s*[A-Za-z_$]')



# A call into another app's command that somebody has looked at and decided to
# leave, with the reason. Empty is the goal. The entry is keyed `app::command`,
# and it exists so this gate can be wired into CI without either lying about the
# two known cases or blocking on a decision it cannot make: a third one fails
# the build, these two stay named.
ACKNOWLEDGED: dict[str, str] = {
    "harness::register_menu": (
        "The harness is arlen-ui's in-flight work, so not ours to move. Named "
        "rather than skipped, because the call is still rejected at runtime."
    ),
    "settings::topbar_items": (
        "The top bar lives in the desktop shell's binary and Settings wants to "
        "list its items. Moving the command into Settings would mean Settings "
        "owning the top bar, and a shared plugin would put a shell-owned surface "
        "in every app - so the answer is neither of this gate's two suggestions "
        "but a cross-binary read, which is a mechanism nobody has chosen yet. "
        "The same is true of the `qs_layout_*` writers and `register_menu`: they "
        "are one decision, not three, and it is not one to invent at 04:30."
    ),
}

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
    acknowledged = []
    backendless: dict[str, list[str]] = {}
    for app in sorted(invokes):
        own = defines.get(app, set())
        for cmd in sorted(invokes[app]):
            if cmd in own or cmd in shared:
                continue
            elsewhere = sorted(a for a, cs in defines.items() if cmd in cs)
            if not elsewhere:
                continue  # nobody defines it: check-invoke-shape's finding, not this one
            if not (ROOT / "apps" / app / "src-tauri").is_dir():
                # A different fact, and the remedy above would be impossible
                # advice: you cannot move a command into an app that has no
                # backend to move it into. The name matching another app's
                # command is a coincidence of vocabulary, not a scoping mistake -
                # every invoke this app makes is a call into nothing, including
                # the ones no app defines, which land in check-invoke-shape's
                # DEAD_INVOKES instead and so look like a different problem.
                backendless.setdefault(app, []).append(cmd)
                continue
            if f"{app}::{cmd}" in ACKNOWLEDGED:
                acknowledged.append(f"{app} invokes `{cmd}`: {ACKNOWLEDGED[f'{app}::{cmd}']}")
                continue
            problems.append(
                f"{app} invokes `{cmd}`, which only {', '.join(elsewhere)} define(s). "
                f"A command lives in one app's binary, so this call is rejected at "
                f"runtime - move it into {app}, or into a shared plugin every app loads"
            )

    # SKIP_PARTS, the same filter the rest of this file uses: a first cut walked
    # the tree raw and reported build output and .svelte-kit chunks, which are
    # copies of the sources already listed. A gate that names generated files is
    # asking someone to go read a file they must not edit.
    wrapped_files = sorted(
        str(f.relative_to(ROOT))
        for f in ROOT.rglob("*")
        if f.suffix in (".ts", ".svelte", ".js")
        and not (SKIP_PARTS & set(f.parts))
        and app_of(f) is not None
        and f.is_file()
        and WRAPPED.search(f.read_text(encoding="utf-8", errors="replace"))
    )
    if wrapped_files:
        print(
            f"{len(wrapped_files)} file(s) route some calls through a local wrapper, "
            "so their command names are not visible here:\n"
            + "\n".join(f"  - {f}" for f in wrapped_files)
            + "\n  A wrapped call into another app's command would not be reported.\n"
        )

    if backendless:
        print("apps with no Tauri backend at all, whose every invoke is a call into nothing:\n")
        for app, cmds in sorted(backendless.items()):
            print(f"  - {app} (no apps/{app}/src-tauri): {', '.join(cmds)}")
        print(
            "    These are counted separately because they are not scoping mistakes.\n"
            "    A frontend built ahead of its backend is a known shape here; a\n"
            "    frontend whose backend was expected to exist is not, and only\n"
            "    someone who knows which can say. Named rather than folded in.\n"
        )

    if acknowledged:
        print("cross-app calls left in place, with a reason:\n")
        for a in acknowledged:
            print(f"  - {a}")
        print()

    if problems:
        print("calls into another app's command:\n")
        for p in problems:
            print(f"  - {p}")
        return 1

    total = sum(len(v) for v in invokes.values())
    # Not "every call resolves in its own app" while `acknowledged` is non-empty:
    # two of them do not, and a summary line that forgets what it just printed is
    # the same defect as a gate that reports and returns success.
    verdict = (
        "every call resolves in its own app"
        if not acknowledged
        else f"{len(acknowledged)} cross-app call(s) left in place with a reason, no new ones"
    )
    print(
        f"{len(invokes)} app(s), {total} distinct invoke(s), "
        f"{len(shared)} shared plugin command(s): {verdict}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
