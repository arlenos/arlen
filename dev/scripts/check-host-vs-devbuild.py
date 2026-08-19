#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""`import.meta.env.DEV` is not the question "is there a backend".

WHAT WENT WRONG. Nine Settings stores decided whether to show a fixture, or
whether to swallow a failed write, by asking `import.meta.env.DEV`. That reads as
"are we in development", and the thing each of them actually needed to know was
"is there a Tauri host to call". Those are different questions and they disagree
in both directions:

  - `tauri dev` is a DEV build WITH a real backend. A command that genuinely
    failed there took the fixture branch: the page showed invented capsules,
    invented printers, an invented grant list, and `appSettings` returned early
    from a failed write so the row kept displaying the value the user typed while
    the file on disk still said the old thing.
  - A release build rendered headlessly - every screenshot drive in
    `dev/screenshot/` - is NOT a DEV build and has no backend either, so it took
    the failure branch and every one of those surfaces reported an error that had
    not happened.

So the same wrong axis produced an invented answer where a real error belonged,
and a real-looking error where nothing was wrong. `$lib/tauri`'s `tauriAvailable`
asks the actual question - it looks for `__TAURI_INTERNALS__` on `window` - and
answers correctly in all four combinations of build mode and host.

WHAT THIS GATE ALLOWS. `import.meta.env.DEV` is fine for things that really are
about the build: a debug log, a dev-only route, a `?nowake` query parameter. It is
refused in a file that also calls `invoke`, because there the branch is about the
backend and the build mode is standing in for it.

The query-parameter case is a carve-out worth naming, because it is the one place
`import.meta.env.DEV` is not only acceptable but REQUIRED. The clock's `?nowake`
and the viewer's `?state=` pin a surface into a state so it can be photographed,
and the gate is the build: a DEV check compiles the branch out of the shipped
bundle, so a released app cannot be talked into displaying a failure that did not
happen. `tauriAvailable` would not do that - a release build with no host would
still honour the parameter. So an occurrence whose own statement reads a query
parameter is left alone.

THE BASELINE. Every app was swept except the harness, whose frontend is another
lane's live work - listing it is how the gate stays green without reaching into it.
Remove the entry when that lane fixes the file. Adding one needs a reason in the
commit.

Run: dev/scripts/check-host-vs-devbuild.py [repo-root]
"""

import pathlib
import re
import sys

ROOT = (
    pathlib.Path(sys.argv[1]).resolve()
    if len(sys.argv) > 1
    else pathlib.Path(__file__).resolve().parents[2]
)

#: Files that still ask the build instead of the host. Shrinking, never growing.
#: Every entry is a surface that shows a fixture under `tauri dev` when its
#: command fails, and reports a failure in a headless release render.
BASELINE = {
    "apps/harness/src/lib/stores/conversation.ts",
}

DEV = "import.meta.env.DEV"
SKIP_DIRS = {"node_modules", "build", ".svelte-kit", "target", "dist"}


def sources(root: pathlib.Path) -> list[pathlib.Path]:
    out = []
    for app in sorted((root / "apps").glob("*/src")):
        for p in app.rglob("*"):
            if p.suffix not in (".ts", ".svelte") or not p.is_file():
                continue
            if SKIP_DIRS & set(p.parts):
                continue
            out.append(p)
    return out


#: A DEV check on a statement that also reads a query parameter is the pin-a-state
#: switch, which the build is the right gate for. See the carve-out above.
QUERY_READ = re.compile(r"searchParams|location\.search")


def asks_the_build(text: str) -> bool:
    """Does this file branch on the build mode while also calling a command?

    Both halves matter. A `console.debug` behind a DEV check is about the build
    and is left alone; the same check in a file that calls `invoke` is standing in
    for "is the backend there", which it cannot answer.

    Occurrences are weighed one at a time rather than by whole file, because a
    file can legitimately hold both: `apps/clock` pins `?nowake` for the screenshot
    loop AND answered a failed daemon read with a fixture, and only the second is
    the defect.
    """
    if DEV not in text:
        return False
    if re.search(r"\binvoke\s*[<(]", text) is None:
        return False
    for line in text.splitlines():
        if DEV in line and not QUERY_READ.search(line):
            return True
    return False


def main() -> int:
    files = sources(ROOT)
    if not files:
        print(f"NOTHING WAS READ: no app sources under {ROOT}/apps", file=sys.stderr)
        return 2

    offenders = set()
    for p in files:
        if asks_the_build(p.read_text(errors="replace")):
            offenders.add(p.relative_to(ROOT).as_posix())

    new = sorted(offenders - BASELINE)
    fixed = sorted(BASELINE - offenders)

    if new:
        print(
            f"{len(files)} app source(s) read, {len(new)} newly ask the build "
            f"instead of the host:\n",
            file=sys.stderr,
        )
        for f in new:
            print(
                f"  - {f}: `{DEV}` decides a branch in a file that calls a "
                f"command. Under `tauri dev` the backend IS there, so a real "
                f"failure takes the fixture path; in a headless release render "
                f"there is no backend and it reports an error that did not "
                f"happen. Ask `tauriAvailable` from `$lib/tauri`.",
                file=sys.stderr,
            )
        return 1

    if fixed:
        print(
            f"{len(fixed)} baseline entr(y/ies) no longer branch on the build. "
            f"Delete them from BASELINE in this file so the line holds:",
            file=sys.stderr,
        )
        for f in fixed:
            print(f"  - {f}", file=sys.stderr)
        return 1

    print(
        f"{len(files)} app source(s) read: outside the {len(BASELINE)} file(s) still "
        f"listed, nothing decides a backend question by asking the build mode. "
        f"`tauri dev` has a backend and a headless release build does not, so "
        f"`import.meta.env.DEV` answers neither."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
