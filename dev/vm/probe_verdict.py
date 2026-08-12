#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Read the knowledge probe's verdict out of a boot journal.

Split out of `verify.py` so it can be shown FAILING. The assertion lived inline,
which meant the only way to exercise it was to boot an image where the graph does
not ingest - and since no such image exists, it had never once been seen to
refuse anything. A gate in that state is indistinguishable from `return True`,
which is the whole reason the standing rule asks for a planted defect.

Three refusals, and the third is the one this exists for:

    no tally       the probe asks twice, 75s apart, and a run that ends before
                   the second round has no verdict at all. "No verdict" must
                   never read as a pass; the natural reading of a silent probe
                   is that nothing went wrong.
    failures       it reported questions it could not ask. Its own count.
    no rows        every question was ANSWERED and every answer was empty. This
                   is the false green: authorised, asked, and the graph held
                   nothing - exactly what a broken ingestion path looks like from
                   outside, and it would otherwise pass with `0 failed`.

Run directly against a rendered journal to see the verdict for a captured boot:

    dev/vm/probe_verdict.py /tmp/boot-journal.log
"""

import re
import sys

# The probe's own gap between rounds, quoted in the failure message so a run that
# lingered too little says what to change rather than only that it failed.
ROUND_GAP_SECONDS = 75


def probe_verdict(journal_text: str) -> tuple[bool, str]:
    """`(ok, message)` for the knowledge probe's lines in `journal_text`."""
    lines = [l for l in journal_text.splitlines() if "kg-probe:" in l]
    tally = [l for l in lines if "question(s) failed" in l]
    rows = [l for l in lines if re.search(r": [1-9][0-9]* row\(s\)", l)]

    if not tally:
        return False, (
            "the knowledge probe never reported a tally. It asks twice "
            f"{ROUND_GAP_SECONDS}s apart, so --linger must reach past that; a probe "
            "with no verdict is not a probe that passed."
        )
    if "done, 0 question(s) failed" not in tally[-1]:
        failed = [l.strip() for l in lines if "FAILED" in l]
        detail = "".join(f"\n    {l}" for l in failed)
        return False, f"the knowledge probe reported failures: {tally[-1].strip()}{detail}"
    if not rows:
        return False, (
            "the knowledge probe was answered but the graph was empty - every "
            "question returned 0 rows. An allowed question with no data is what a "
            "broken ingestion path looks like from here."
        )
    return True, (
        f"{tally[-1].split('kg-probe:')[-1].strip()}, "
        f"{len(rows)} question(s) returned rows"
    )


def main() -> int:
    if len(sys.argv) != 2:
        print(f"usage: {sys.argv[0]} <rendered-journal>", file=sys.stderr)
        return 2
    with open(sys.argv[1], encoding="utf-8", errors="replace") as fh:
        ok, message = probe_verdict(fh.read())
    print(("knowledge probe: " if ok else "VERIFY FAIL: ") + message)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
