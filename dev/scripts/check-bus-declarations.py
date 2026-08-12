#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A component on the event bus must say what it HEARS, not only what it sends.

Omitting `subscribe` is not neutral. The bus reads an absent key as "never
bounded" and lets a system-tier peer keep the machine-wide view its path gives
it, so a profile that declares only `publish` is a component that publishes on
the record and hears everything off it. That asymmetry is exactly how the
compositor stayed invisible: it emitted window events nobody had authorised and
would have been handed the subscribe side too, because one tier label carried
both effects.

So the rule is: if you are on the bus, declare both halves. An empty
`subscribe = []` is a fine answer and the right one for a pure producer - it says
"hears nothing" rather than saying nothing, and the SDK keeps those two apart
(`declares_subscribe`). What this refuses is silence.

The check is deliberately narrow. It does not judge the CONTENTS of either list -
whether the compositor should hear `config.changed` is a design question, and a
script with an opinion about it would be wrong within a week. It judges only that
the question was answered somewhere a reader can find it.
"""

import pathlib
import re
import sys

ROOT = (
    pathlib.Path(sys.argv[1]).resolve()
    if len(sys.argv) > 1
    else pathlib.Path(__file__).resolve().parents[2]
)
PROFILES = ROOT / "dev/mkosi/mkosi.extra/var/lib/arlen/permissions"

# Components that participate in the bus and deliberately keep the unbounded
# view. Empty on purpose today: every shipped participant declares both halves,
# and an entry here should have to argue for itself.
UNBOUNDED: dict[str, str] = {}


def main() -> int:
    if not PROFILES.is_dir():
        print(f"NOTHING WAS READ: no profiles under {PROFILES}", file=sys.stderr)
        return 2

    profiles = sorted(PROFILES.rglob("*.toml"))
    if not profiles:
        print(f"NOTHING WAS READ: no profile files under {PROFILES}", file=sys.stderr)
        return 2

    problems: list[str] = []
    participants = 0
    for path in profiles:
        text = path.read_text()
        # Section-scoped: `subscribe` under [event_bus], not a key of that name
        # anywhere in the file. A profile with an unrelated `subscribe` elsewhere
        # would otherwise pass while saying nothing about the bus.
        section = re.search(r"^\[event_bus\]\n(.*?)(?=^\[|\Z)", text, re.M | re.S)
        if not section:
            continue
        participants += 1
        name = path.stem
        body = section.group(1)
        for half in ("publish", "subscribe"):
            if re.search(rf"^{half}\s*=", body, re.M):
                continue
            if name in UNBOUNDED:
                continue
            problems.append(
                f"{name} is on the event bus and never declares `{half}`. "
                f"An absent `subscribe` is read as unbounded rather than as none, "
                f"so write `{half} = []` if the answer is nothing - or add {name} "
                f"to UNBOUNDED with the reason it keeps the wider view."
            )

    for stale in sorted(UNBOUNDED):
        if not (PROFILES / f"{stale}.toml").exists() and not any(
            p.stem == stale for p in profiles
        ):
            problems.append(f"{stale} is excused here and ships no profile; delete the entry")

    if problems:
        print("event-bus declarations:\n")
        for p in problems:
            print(f"  - {p}")
        return 1

    print(
        f"{participants} event-bus participant(s); each declares both what it "
        f"publishes and what it hears"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
