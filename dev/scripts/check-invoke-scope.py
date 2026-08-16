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

Where this runs, stated carefully because the previous version of this paragraph
was out of date and the replacement I wrote for it inherited the gap.

It was held out of the gate list on the grounds that it reported two real
pre-existing calls, and declaring them green would be the wrong way round. That
stopped being the whole picture at `4c0445b6c`, which put it in CI with those two
named in `ACKNOWLEDGED` - so it has been running on every push since, while this
docstring still said to run it by hand. Both calls have since been fixed anyway,
each app having grown its own command of that name forwarding to the shell, so the
list is empty because the calls resolve rather than because anything was declared.
Added to `just checks` and given a positive control on 12 Aug, which is what
changed that day - not whether it runs.
"""

from collections.abc import Callable
import pathlib
import re
import sys

# The tree to scan. An argument so this can be pointed at a fixture and shown
# to fail: a check that only ever runs against a tree that already passes
# cannot demonstrate the defect it exists for (standing rule, 11 Aug).
ROOT = (
    pathlib.Path(sys.argv[1]).resolve()
    if len(sys.argv) > 1
    else pathlib.Path(__file__).resolve().parents[2]
)
SKIP_PARTS = {"target", "node_modules", ".git", "build", ".svelte-kit", "mkosi.builddir"}
# `mkosi.builddir` is the image build cache. It is gitignored, so CI never sees it,
# but it holds a cargo checkout of an OLDER commit of this repo - 45k Rust files that
# this walk was reading on every local run. Findings there point at source nobody can
# edit, dated to whatever commit the image last built from, and the read cost a minute
# of every pre-commit.


COMMAND = re.compile(r"#\[tauri::command[^\]]*\]\s*(?:pub\s+)?(?:async\s+)?fn\s+(\w+)")
INVOKE = re.compile(r'invoke(?:<[^>]*>)?\(\s*"([a-z_][a-z0-9_]*)"')
# A call made through a local helper - `send(cmd, args)` wrapping `invoke(cmd,
# args)` - carries no literal for the pattern above to find, so it is invisible
# here exactly as it was in `check-invoke-shape`, where the clock's fifteen
# actions were being counted past. This check asks a different question of the
# same calls (whose binary defines them), so the blind spot means a wrapped call
# into ANOTHER app's command would not be reported at all.
#
# This said "named in the summary rather than resolved: following a wrapper can
# be confidently wrong", which was true when nothing followed one. It is now
# done in `check-invoke-exists.py` with a control on both directions - a wrapped
# call is seen, and a typo inside one fails - so the reason has expired rather
# than the risk having been argued away. The name is safe to resolve for the same
# reason it was there: a literal at a call site of a helper that passes its own
# first parameter to `invoke` arrives as `invoke`'s first argument, so it is a
# command name. Whose binary defines it is then the ordinary question.
WRAPPER = re.compile(
    r"(?:async\s+)?function\s+(\w+)\s*\(\s*(\w+)[^)]*\)[^{]*\{[^}]*?\binvoke\s*\(\s*\2\b",
    re.S,
)
WRAPPED = re.compile(r'\binvoke\s*(?:<[^>]*>)?\s*\(\s*[A-Za-z_$]')


def wrapped_calls(body: str) -> set[str]:
    """Command names passed to a local helper that forwards them to `invoke`."""
    out: set[str] = set()
    for wm in WRAPPER.finditer(body):
        helper = wm.group(1)
        for m in re.finditer(rf"\b{re.escape(helper)}\s*\(\s*(\"[^\"]+\"|'[^']+')", body):
            out.add(m.group(1)[1:-1])
    return out



# A call into another app's command that somebody has looked at and decided to
# leave, with the reason. Empty is the goal, and it is empty.
#
# It held two - `harness::register_menu` and `settings::topbar_items` - each
# saying the call was still rejected at runtime and that the fix needed a
# cross-binary mechanism nobody had chosen. Both stopped being true without
# either entry noticing: each app grew its OWN command of that name
# (`apps/harness/src-tauri/src/menu.rs`, `apps/settings/src-tauri/src/commands/
# topbar.rs`), which is a local conduit forwarding to the shell, so the calls
# resolve in their own binary and the gate had quietly stopped reporting them.
# The excuses outlived their subject by however long that took, and nothing said
# so, because this list had no staleness guard - the one thing `check-wired.py`
# does to its own exemptions and the reason it caught its author twice in an
# evening. Cleared on 12 Aug, and the guard below is why it cannot happen again.
ACKNOWLEDGED: dict[str, str] = {}

def app_of(path: pathlib.Path) -> str | None:
    """The app a file belongs to, or `None` for anything outside `apps/`."""
    parts = path.relative_to(ROOT).parts
    return parts[1] if len(parts) > 2 and parts[0] == "apps" else None


def scan(
    extract: Callable[[str], set[str]], suffixes: tuple[str, ...]
) -> dict[str, set[str]]:
    """Every name `extract` finds in a file, grouped by the app it belongs to.

    Takes a function rather than a pattern because one of the three things read
    here - a name handed to a wrapper - needs two passes over the same text to
    find, and a caller that can only be a regex would have left that one out.
    """
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
        found = extract(body)
        if found:
            out.setdefault(app, set()).update(found)
    return out


def matches(pattern: re.Pattern[str]) -> Callable[[str], set[str]]:
    """`scan` extractor for a plain first-group pattern."""
    return lambda body: {m.group(1) for m in pattern.finditer(body)}


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
    defines = scan(matches(COMMAND), (".rs",))
    front = (".ts", ".svelte", ".js")
    invokes = scan(
        lambda body: matches(INVOKE)(body) | wrapped_calls(body), front
    )
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

    # The other direction, which is the one that had rotted: an acknowledgement
    # for a call this no longer reports. The entry claims a defect is still there
    # and being tolerated; once the call resolves, that sentence is false, and a
    # false excuse is worse than no excuse because it reads as remaining work
    # somebody owes. Both entries this list started with had reached that state.
    seen = {f"{app}::{cmd}" for app in invokes for cmd in invokes[app]}
    for key in sorted(ACKNOWLEDGED):
        app, _, cmd = key.partition("::")
        if key not in seen:
            problems.append(
                f"`{key}` is acknowledged as a cross-app call, but {app} no longer "
                f"invokes `{cmd}` at all. Drop the entry."
            )
        elif cmd in defines.get(app, set()) or cmd in shared:
            problems.append(
                f"`{key}` is acknowledged as a cross-app call, but {app} now defines "
                f"`{cmd}` itself (or a shared plugin does). The reason it names has "
                f"been overtaken; drop the entry."
            )

    # SKIP_PARTS, the same filter the rest of this file uses: a first cut walked
    # the tree raw and reported build output and .svelte-kit chunks, which are
    # copies of the sources already listed. A gate that names generated files is
    # asking someone to go read a file they must not edit.
    #
    # Only the files this cannot RESOLVE are worth naming. A wrapper whose call
    # sites carry literals is followed now, so listing it would report a blind
    # spot that has been closed - and a list that names things already handled is
    # how a list stops being read. What is left is `invoke` reached by a name this
    # cannot trace to a literal at all: built at runtime, or passed down through
    # something other than the one-hop helper shape.
    unresolved = []
    for f in ROOT.rglob("*"):
        if (
            f.suffix not in (".ts", ".svelte", ".js")
            or SKIP_PARTS & set(f.parts)
            or app_of(f) is None
            or not f.is_file()
        ):
            continue
        body = f.read_text(encoding="utf-8", errors="replace")
        if WRAPPED.search(body) and not wrapped_calls(body):
            unresolved.append(str(f.relative_to(ROOT)))
    if unresolved:
        print(
            f"{len(unresolved)} file(s) reach `invoke` by a name this cannot trace "
            "to a literal:\n"
            + "\n".join(f"  - {f}" for f in sorted(unresolved))
            + "\n  A call into another app's command from one of these is not "
            "reported.\n"
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
