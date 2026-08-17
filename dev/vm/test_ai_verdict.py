#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Controls for `ai_verdict`: the boots it must refuse, and one it must not.

The no-event case is not hypothetical. `--require-dogfood` accepted it until 16
August, because it gated on the terminal marker alone and the dogfood prints that
whatever happened.

The last case is the other half of the same lesson, and it is a real captured
boot rather than a constructed one - `dev/vm/serial-consent-check.log`, emit ok,
ask skipped, terminal marker printed. My first cut refused it, which reads right
from the flag's help text and is wrong: that ask is best-effort by design and is
currently refused by an admission decision nobody has made, so a gate on it would
turn every boot red over an open question rather than a defect. It must PASS, and
it must still say the completion did not come back.

Run: python3 dev/vm/test_ai_verdict.py
"""

import sys

from ai_verdict import ai_verdict

# A boot where everything worked. The stage lines are copied from the shapes
# `dev/dogfood/src/main.rs` prints, so a rename there fails these rather than
# passing quietly against markers nothing emits any more.
GOOD = """
[   4.512331] arlen-dogfood[612]: DOGFOOD EMIT ok path=/var/lib/arlen-work/notes.md
[   9.884120] arlen-dogfood[612]: DOGFOOD ASK ok answer=The file notes.md was opened
[  10.121004] arlen-dogfood[612]: DOGFOOD OK
"""

# A real captured boot: emit ok, the ask refused, terminal marker printed. Copied
# from `dev/vm/serial-consent-check.log` rather than invented, so it stays a boot
# that actually happened.
ASK_SKIPPED_BOOT = """
[   4.512331] arlen-dogfood[632]: DOGFOOD EMIT ok path=/var/lib/arlen-work/notes.md
[  73.190133] arlen-dogfood[632]: DOGFOOD ASK skipped (best-effort, 1B model): explain_system: org.freedesktop.DBus.Error.UnknownMethod: Unknown method 'explain_system'
[  73.900021] arlen-dogfood[632]: DOGFOOD OK
"""

CASES = [
    (
        "no event was injected, so the completion is not about this boot",
        """
[   9.884120] arlen-dogfood[612]: DOGFOOD ASK ok answer=Something from an older graph
[  10.121004] arlen-dogfood[612]: DOGFOOD OK
""",
    ),
    (
        "the probe died before finishing",
        """
[   4.512331] arlen-dogfood[612]: DOGFOOD EMIT ok path=/var/lib/arlen-work/notes.md
[   6.223110] arlen-dogfood[612]: DOGFOOD FAIL could not reach the agent socket
""",
    ),
    (
        "the probe said nothing at all",
        "[   4.512331] systemd[1]: Started arlen-dogfood.service.\n",
    ),
    (
        "an empty journal",
        "",
    ),
]


def main() -> int:
    failures = []

    ok, message = ai_verdict(GOOD)
    if not ok:
        failures.append(f"a good boot was refused: {message}")

    for name, journal in CASES:
        ok, message = ai_verdict(journal)
        if ok:
            failures.append(f"PASSED a boot that must be refused: {name}")
        elif not message.strip():
            failures.append(f"refused {name!r} with an empty message")

    # A boot whose ask was skipped PASSES, and must still report that it was: the
    # gate not refusing is not the same as the verdict going quiet. The reason is
    # quoted verbatim because the last summary written in this area named the
    # wrong cause for weeks.
    ok, skip_message = ai_verdict(ASK_SKIPPED_BOOT)
    if not ok:
        failures.append(f"refused a boot whose ask was best-effort skipped: {skip_message}")
    if "best-effort" not in skip_message or "UnknownMethod" not in skip_message:
        failures.append(f"the skip verdict dropped the probe's stated reason: {skip_message}")

    # A journal that never reached this side must not be read as a dogfood that
    # failed. `verify.py` turns an unreadable serial log into the empty string,
    # so this is the shape that actually arrives, not a hypothetical one.
    for empty, what in (("", "an empty journal"), ("some unrelated line\n", "a journal with no DOGFOOD line")):
        ok, message = ai_verdict(empty)
        if ok:
            failures.append(f"{what} passed")
        elif "measured nothing" not in message:
            failures.append(f"{what} was blamed on the dogfood: {message}")

    for f in failures:
        print(f"FAIL: {f}", file=sys.stderr)
    if failures:
        return 1
    print(
        f"ok: a good boot and a best-effort skip pass, {len(CASES)} bad ones are refused, "
        "and a journal that never arrived is not called a failure"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
