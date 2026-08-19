#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Check that the calendar has not grown a mail path of its own.

WHY. `mail-app.md` section 4 leaves one architectural question open and settles one
thing about it. The open question is who owns iTIP processing - the mail daemon or
the calendar daemon - and the plan says it got no verified evidence, so it is a call
to make on purpose rather than by default. The settled thing is the guard rail:
**the calendar must not grow a partial SMTP path in the meantime.**

That rail is exactly the kind that gets crossed by an obvious-looking edit. Somebody
implements RSVP, needs to send one message, adds `lettre` to the calendar daemon, and
the architectural question is now answered - by whoever was closest to it, in a commit
about something else. The decision would never be discussed; it would just be true.

So this reads the calendar's manifests and sources for a mail-sending path and fails
if one appears. It is not a security control and does not pretend to be: a determined
implementation could shell out to `sendmail` and this would not see it. It is a
tripwire on the specific, likely, undramatic edit that would decide an open question
silently.

WHAT COUNTS. A mail-transport dependency in a calendar manifest, or SMTP protocol
verbs in calendar source. Reading an invitation is NOT a mail path - the calendar
already does that, deliberately, and stops where sending begins.

Run: dev/scripts/check-calendar-has-no-mail-path.py [root]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

#: The calendar's own trees. Its app and its daemon both count: the rail is about
#: the calendar as a component, not about one crate in it.
CALENDAR = ["apps/calendar", "daemons/calendar"]

#: Crates that send mail. Named rather than pattern-matched, because the point is
#: to catch the specific edit that adds one of these to the calendar.
MAIL_CRATES = ["lettre", "mail-send", "async-smtp", "samotop", "sendmail"]

#: SMTP the protocol, as it appears in code that speaks it. `EHLO` and `STARTTLS`
#: are unambiguous; a bare "MAIL FROM" needs the colon to avoid matching prose.
SMTP_VERBS = re.compile(r"\b(EHLO|STARTTLS|MAIL FROM:|RCPT TO:)", re.IGNORECASE)


def manifests(base: Path) -> list[Path]:
    """Every Cargo.toml under a calendar tree."""
    return sorted(base.rglob("Cargo.toml")) if base.is_dir() else []


def sources(base: Path) -> list[Path]:
    """Every Rust and TypeScript source under a calendar tree, tests excluded.

    A test naming SMTP to assert its absence is not a mail path, and failing on
    one would make this check impossible to write a control for.
    """
    if not base.is_dir():
        return []
    out: list[Path] = []
    for pattern in ("*.rs", "*.ts", "*.svelte"):
        out.extend(
            p
            for p in base.rglob(pattern)
            if "target" not in p.parts and "node_modules" not in p.parts and "tests" not in p.parts
        )
    return sorted(out)


def main() -> int:
    problems: list[str] = []
    read = 0

    for rel in CALENDAR:
        base = ROOT / rel
        for manifest in manifests(base):
            text = manifest.read_text(encoding="utf-8", errors="replace")
            read += 1
            for crate in MAIL_CRATES:
                if re.search(rf'^\s*{re.escape(crate)}\s*=', text, re.M):
                    problems.append(
                        f"{manifest.relative_to(ROOT)} depends on `{crate}`, which sends mail.\n"
                        f"    mail-app.md section 4: the calendar must not grow a partial SMTP path "
                        f"while who owns iTIP is still an open call. Put the sending in the mail "
                        f"component, or make the architectural decision on purpose and update the plan."
                    )
        for src in sources(base):
            text = src.read_text(encoding="utf-8", errors="replace")
            read += 1
            if match := SMTP_VERBS.search(text):
                line = text[: match.start()].count("\n") + 1
                problems.append(
                    f"{src.relative_to(ROOT)}:{line} speaks SMTP (`{match.group(0)}`).\n"
                    f"    Same rail: the calendar reads invitations and stops where sending begins."
                )

    if read == 0:
        print(
            "NOTHING WAS READ: no calendar sources or manifests found, so this checked nothing",
            file=sys.stderr,
        )
        return 2

    if problems:
        print("the calendar has grown a mail path:\n")
        for p in problems:
            print(f"  - {p}")
        return 1

    print(f"{read} calendar file(s) read; none sends mail.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
