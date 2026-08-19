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

THE BASELINE. The sweep landed in Settings, Files and Knowledge, where the DEV
fixtures were being read as real. The files below still carry the old shape;
they are listed below so this gate can hold the line from today rather than wait
for one large change nobody can verify in a sitting. Remove an entry when you fix
the file. Adding one needs a reason in the commit.

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
    "apps/clock/src/lib/stores/clock.ts",
    "apps/desktop-shell/src/lib/components/topbar/badges/CaptureBadge.svelte",
    "apps/desktop-shell/src/lib/components/topbar/badges/DictationBadge.svelte",
    "apps/desktop-shell/src/lib/stores/activePopover.ts",
    "apps/desktop-shell/src/lib/stores/bluetoothPairing.ts",
    "apps/desktop-shell/src/lib/stores/consent.ts",
    "apps/desktop-shell/src/lib/stores/jobs.ts",
    "apps/desktop-shell/src/lib/stores/printDialog.ts",
    "apps/desktop-shell/src/lib/stores/sourcePicker.ts",
    "apps/desktop-shell/src/lib/stores/waypointerAsk.ts",
    "apps/desktop-shell/src/lib/stores/windowsFile.ts",
    "apps/harness/src/lib/stores/conversation.ts",
    "apps/meetings/src/lib/stores/meeting.ts",
    "apps/system-monitor/src/lib/stores/processes.ts",
    "apps/text-editor/src/lib/stores/aiEdit.ts",
    "apps/viewers/src/routes/+page.svelte",
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


def asks_the_build(text: str) -> bool:
    """Does this file branch on the build mode while also calling a command?

    Both halves matter. A `console.debug` behind a DEV check is about the build
    and is left alone; the same check in a file that calls `invoke` is standing in
    for "is the backend there", which it cannot answer.
    """
    if DEV not in text:
        return False
    return re.search(r"\binvoke\s*[<(]", text) is not None


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
        f"{len(files)} app source(s) read: no file outside the {len(BASELINE)} "
        f"known ones decides a backend question by asking the build mode. "
        f"`tauri dev` has a backend and a headless release build does not, so "
        f"`import.meta.env.DEV` answers neither."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
