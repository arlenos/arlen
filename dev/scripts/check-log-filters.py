#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""An app's log filter must name its own crate, not set a level for everything.

Two defects came out of one line, in thirteen apps, and neither was anyone's
decision:

    default_filter_or("info")   sets info for every crate in the process, so zbus
                                logs its D-Bus handshake frames WITH the message
                                bytes. A message body is user content - file
                                paths, query strings, notification text - and at
                                info it lands in a journal that no capability
                                grant covers. That is the graph's whole scoping
                                story undone in a log line.
    env_logger::init()          defaults to `error`, so the app is mute: every
                                log::info! and log::warn! in it produces nothing.
                                This is what made the boot consent hang so hard
                                to find - the component in the middle could not
                                be heard - and it was true of four apps at once.

The shape that avoids both is `warn,<own_crate>=info`: the app speaks, its
dependencies do not. So the rule is that the filter must MENTION a crate. A bare
level, in either direction, is the thing that produced both defects.

It does not check WHICH crate or which levels - `warn,x=info` and
`error,x=debug,y=trace` are both somebody's considered choice. It refuses only
the blanket, because a blanket is what nobody chooses and everybody inherits.
"""

import pathlib
import re
import sys

ROOT = (
    pathlib.Path(sys.argv[1]).resolve()
    if len(sys.argv) > 1
    else pathlib.Path(__file__).resolve().parents[2]
)
APPS = ROOT / "apps"

# app -> why its filter is not ours to fix.
NOT_OURS: dict[str, str] = {
    "store": (
        "arlen-ui's live work; the same blanket `info` is there and the fix is the "
        "same one line, but editing their tree mid-flight is worse than the defect"
    ),
}


def _code_only(text: str) -> str:
    """Drop `//` line comments before matching.

    Without this the check reads its own advice back as a defect: the fix it asks
    for is usually written up in a comment right above the call, quoting the bad
    form to explain why it was replaced. Scanning that as code made all four
    already-fixed apps fail - the same "counted the wrong thing" mistake as the
    fixture guard, one night apart.
    """
    return "\n".join(re.sub(r"//.*$", "", line) for line in text.splitlines())


def main() -> int:
    if not APPS.is_dir():
        print(f"NOTHING WAS READ: no apps directory under {ROOT}", file=sys.stderr)
        return 2

    sources = sorted(APPS.glob("*/src-tauri/src/*.rs"))
    if not sources:
        print(f"NOTHING WAS READ: no app sources under {APPS}", file=sys.stderr)
        return 2

    problems: list[str] = []
    checked = 0
    for app_dir in sorted(APPS.iterdir()):
        src = app_dir / "src-tauri" / "src"
        if not src.is_dir():
            continue
        app = app_dir.name
        text = "\n".join(_code_only(p.read_text()) for p in sorted(src.glob("*.rs")))
        if "env_logger" not in text and "tracing_subscriber" not in text:
            continue
        checked += 1
        if app in NOT_OURS:
            continue

        bare_init = re.search(r"\benv_logger::init\(\)", text)
        blanket = re.search(r'default_filter_or\(\s*"(trace|debug|info|warn|error)"\s*\)', text)
        if bare_init:
            problems.append(
                f"{app}: `env_logger::init()` defaults to `error`, so this app is mute "
                f"in the journal. Use `default_filter_or(\"warn,<its_crate>=info\")`."
            )
        elif blanket:
            problems.append(
                f"{app}: `default_filter_or(\"{blanket.group(1)}\")` sets a level for "
                f"EVERY crate in the process, dependencies included - which is how "
                f"zbus message bytes reached the journal. Name the app's own crate: "
                f"`\"warn,<its_crate>=info\"`."
            )

    # Only against the real tree. `NOT_OURS` names apps in THIS repo, so checking
    # it against a fixture asks whether a made-up directory contains `store` - and
    # the answer is always no, which failed every fixture the control planted. An
    # excuse list is a claim about one tree; validating it elsewhere is a category
    # error, and the control is what surfaced it.
    if len(sys.argv) <= 1:
        for stale in sorted(NOT_OURS):
            if not (APPS / stale).is_dir():
                problems.append(f"{stale} is excused here and no longer exists; delete the entry")

    if problems:
        print("app log filters:\n")
        for p in problems:
            print(f"  - {p}")
        return 1

    print(
        f"{checked} app(s) initialise logging; each names its own crate rather than "
        f"setting a level for every dependency ({len(NOT_OURS)} excused)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
