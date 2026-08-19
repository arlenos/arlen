#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Check that every app in `apps/` is either staged into the image or says why not.

WHY THIS EXISTS. On 15 August the viewer was opened on a real video to see what the
video face did, and the answer turned out to be that the viewer is not on the machine
at all: no build step, therefore no binary, no desktop entry and no MIME registration,
so the file manager could not launch it and the plan's premise had never been true.
Nothing said so. The app built, its tests passed, its screenshots looked right, and it
existed only on the developer's host.

That is a whole class - an app that is finished and absent - and it is invisible from
either side on its own. The build steps look complete because each one that exists
works; `apps/` looks complete because each app is really there. Only the two together
show the gap, which is what this reads.

An app counts as staged when a `dev/mkosi/mkosi.build.d/*.chroot` step both reads its
source as `$SRCDIR/arlen/apps/<name>` and installs into `/usr/lib/arlen/apps/`. Both
halves earn their place. Without the install, a step that only builds an app would
vouch for it; and matching a bare `apps/<name>` rather than the `$SRCDIR` form let one
sentence in a comment - "see also apps/viewers, which is similar" - make this check
green for an app nobody ships. The control found that in this file, which is the whole
argument for writing controls that watch a check fail.

Everything else must be in NOT_ON_IMAGE with a reason. The reason is the point: "not
yet" is a note, "the image autologs in and has no greeter UI" is a decision somebody
can disagree with. Write the sentence that would let a reader tell which one it is.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

# `--excused` prints the NOT_ON_IMAGE names and exits. The control needs them to
# build a tree this check considers well-formed, and hardcoding the list there would
# make the two drift apart the first time an app is staged - so the check answers for
# itself instead.
ARGS = [a for a in sys.argv[1:] if a != "--excused"]
LIST_ONLY = "--excused" in sys.argv[1:]
ROOT = Path(ARGS[0]).resolve() if ARGS else Path(__file__).resolve().parents[2]
APPS = ROOT / "apps"
STEPS = ROOT / "dev/mkosi/mkosi.build.d"

# App directory to the reason it is not on the image. Each was checked on 15 August
# rather than assumed, and each says which kind of absence it is.
NOT_ON_IMAGE: dict[str, str] = {
    "greeter": (
        "the image autologs in and has no greeter UI - `mkosi.extra/etc/greetd/config.toml` "
        "runs arlen-session for both the initial and the default session, and says so. "
        "The app is real; shipping it is a decision about whether this image stays a "
        "single-user appliance, not an oversight"
    ),
    "mail": (
        "one file old: the stated rule for the two MIME headers the standard left "
        "ambiguous, which is where `mail-app.md` section 2 starts because a parser "
        "with no written rule has an accidental one. There is no client yet - no "
        "IMAP, no store, no window - so there is nothing to stage. This entry goes "
        "when there is an app rather than a rule"
    ),
    "pdf": (
        "the reader is real now - it opens a document, shows the author's contents, "
        "searches inside it, turns pages and selects text, and it starts under the "
        "app smoke. What it cannot do on any machine yet is DRAW a page: pdfium-render "
        "binds to a `libpdfium` at runtime and no distribution in play ships one as a "
        "package, so staging it would put an app on the image that shows every document "
        "as a blank sheet. It ships when the engine has a home - a built package, a "
        "checksummed binary at image-build time, or PDFium from source in the pipeline, "
        "which is a supply-chain call rather than a coder's"
    ),
    "calendar": (
        "new on 19 August and one sitting old: it reads the .ics files in "
        "`$XDG_DATA_HOME/arlen/calendars` and shows an agenda, which is worth "
        "shipping, but nothing writes those files yet - no sync, no import, no "
        "way to add an event - so on the image today it would be an app that is "
        "empty for everyone. It ships when something puts a calendar there"
    ),
    "settings": (
        "arlen-ui's live work; the coder does not stage it. It is the largest app with no "
        "image step (49 Rust files, 71 components), so if it is meant to be reachable on "
        "the machine this is the biggest single gap in the set"
    ),
    "harness": "arlen-ui's live work, and the surface they are redesigning",
    "store": "arlen-ui's live work",
    "trash-rm": (
        "a command-line tool (`arlen-trash-rm`), not a desktop app, so it has no launcher "
        "entry by nature. It still belongs in `/usr/bin` if the trash flow is meant to work "
        "from a shell, which is a separate question from this check"
    ),
    "wine-manager": "an empty placeholder: the directory holds a .gitignore and nothing else",
}

INSTALLS_APP = re.compile(r"\$DESTDIR/usr/lib/arlen/apps/")


def is_app(d: Path) -> bool:
    """A directory under `apps/` that is an application rather than scaffolding.

    Either a Tauri app (`src-tauri/`) or a Rust binary crate. A directory holding
    only a .gitignore is neither, and is listed in NOT_ON_IMAGE anyway so that
    deleting it is a decision somebody makes on purpose.
    """
    if not d.is_dir() or d.name.startswith("."):
        return False
    if (d / "src-tauri").is_dir():
        return True
    return any(d.glob("*/Cargo.toml")) or (d / "Cargo.toml").is_file()


def staged() -> dict[str, str]:
    """App directory name to the build step that stages it."""
    out: dict[str, str] = {}
    for step in sorted(STEPS.glob("*.chroot")):
        text = step.read_text(encoding="utf-8", errors="replace")
        if not INSTALLS_APP.search(text):
            continue
        for m in re.finditer(r"\$SRCDIR/arlen/apps/([a-z0-9-]+)", text):
            out.setdefault(m.group(1), step.name)
    return out


def main() -> int:
    if LIST_ONLY:
        for name in sorted(NOT_ON_IMAGE):
            print(name)
        return 0
    if not APPS.is_dir() or not STEPS.is_dir():
        print("apps/ or the build steps are missing; the layout moved and this check did not")
        return 1

    apps = {d.name for d in APPS.iterdir() if is_app(d)}
    # wine-manager holds only a .gitignore, so `is_app` says no - but it is named in
    # NOT_ON_IMAGE and that entry should not rot silently either.
    apps |= {name for name in NOT_ON_IMAGE if (APPS / name).is_dir()}
    if not apps:
        print("no apps found; the layout moved and this check did not")
        return 1

    on_image = staged()
    problems: list[str] = []

    for app in sorted(apps):
        if app in on_image:
            if app in NOT_ON_IMAGE:
                problems.append(
                    f"{app} IS staged by {on_image[app]} and is still listed in "
                    f"NOT_ON_IMAGE. Remove the entry: a stale excuse is worse than none, "
                    f"because it says the gap is known when it is closed"
                )
            continue
        if app not in NOT_ON_IMAGE:
            problems.append(
                f"{app} is an app in apps/ with no build step, so it is not on the image: "
                f"no binary, no desktop entry, no MIME registration, and nothing on the "
                f"machine can launch it. Add a dev/mkosi/mkosi.build.d step, or add it to "
                f"NOT_ON_IMAGE with the reason"
            )

    for name in sorted(NOT_ON_IMAGE):
        if not (APPS / name).is_dir():
            problems.append(
                f"{name} is listed in NOT_ON_IMAGE and no longer exists in apps/; "
                f"drop the entry"
            )

    if problems:
        print("\napps and the image disagree:\n")
        for p in problems:
            print(f"  - {p}")
        return 1

    print(
        f"{len(apps)} app(s): {len(on_image.keys() & apps)} staged into the image, "
        f"{len(apps) - len(on_image.keys() & apps)} absent with a stated reason. An app that is "
        f"finished and absent is invisible from either side alone, which is why both are "
        f"read here."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
