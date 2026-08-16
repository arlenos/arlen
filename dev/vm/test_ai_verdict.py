#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Controls for `ai_verdict`, each a boot that must not pass as an AI boot.

The first case is not hypothetical. It is what `--require-ai` accepted until 16
August: every stage line absent except the terminal marker, which the dogfood
prints whatever happened.

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

CASES = [
    (
        "the ask was skipped and the terminal marker printed anyway",
        """
[   4.512331] arlen-dogfood[612]: DOGFOOD EMIT ok path=/var/lib/arlen-work/notes.md
[   9.884120] arlen-dogfood[612]: DOGFOOD ASK skipped (best-effort): the call succeeded and returned an empty answer
[  10.121004] arlen-dogfood[612]: DOGFOOD OK
""",
        # The exact false green. The old gate saw DOGFOOD OK and passed a boot
        # whose AI layer answered nothing at all.
    ),
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

    # The skip case must name the probe's own reason rather than a summary of it.
    # The last summary in this area named the wrong cause for weeks, which is why
    # the dogfood prints the reason verbatim; a verdict that drops it undoes that.
    _, skip_message = ai_verdict(CASES[0][1])
    if "empty answer" not in skip_message:
        failures.append("the skip verdict dropped the probe's stated reason")

    for f in failures:
        print(f"FAIL: {f}", file=sys.stderr)
    if failures:
        return 1
    print(f"ok: a good boot passes and {len(CASES)} bad ones are refused")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
