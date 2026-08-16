#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Read the dogfood's AI verdict out of a boot journal.

Split out of `verify.py` for the reason `probe_verdict.py` was, and with a
sharper version of the same defect underneath it.

`--require-dogfood` says, in its own help text, that the journal "must show
'DOGFOOD OK' (event injected + AI completion)". It gated on `DOGFOOD OK` alone,
and the probe prints that line unconditionally at the end of `main`. The ask step is
deliberately best-effort: when no completion comes back it prints `DOGFOOD ASK
skipped (best-effort)` and falls through to `DOGFOOD OK` anyway, by design, so
the same binary can run where no model is provisioned.

So the flag asserted a completion it never checked. A boot where the AI layer
answered nothing passed `--require-dogfood`, and the stage lines that would have
said so were printed for a human to read and not gated on. This is deepseek's rule 1
in our own tree: the harness trusted the agent's summary of itself instead of
the thing the summary is about.

The stages are separate facts and are kept separate here - the same reason their
rule 2 asks for `timedOut`, `signal` and `exitCode` side by side. An umbrella
marker that outlives its stages is not a verdict about them.

Three refusals:

    never finished   no terminal marker at all: the run died, or ended before the
                     probe did. The probe's own `DOGFOOD FAIL` line is quoted when
                     there is one, because it names the cause and this cannot.
    no event         `DOGFOOD EMIT ok` absent. The first half of what the flag
                     claims: nothing was injected, so whatever else happened was
                     not about this boot.
    no completion    `DOGFOOD OK` is there and `DOGFOOD ASK ok` is not. The false
                     green this exists for. The probe's skip line carries the
                     reason verbatim, so it is quoted rather than summarised.

Run directly against a rendered journal to see the verdict for a captured boot:

    dev/vm/ai_verdict.py /tmp/boot-journal.log
"""

import sys

# The dogfood's stage markers. Kept in step with `dev/dogfood/src/main.rs`, which
# prints them one per stage and `DOGFOOD OK` at the end whatever happened.
EMIT_OK = "DOGFOOD EMIT ok"
ASK_OK = "DOGFOOD ASK ok"
ASK_SKIPPED = "DOGFOOD ASK skipped"
TERMINAL_OK = "DOGFOOD OK"
FAIL_MARKER = "DOGFOOD FAIL"


def _quoted(journal_text: str, marker: str) -> str:
    """The first line carrying `marker`, indented for a message, or nothing."""
    for line in journal_text.splitlines():
        if marker in line:
            return f"\n    {line.strip()}"
    return ""


def ai_verdict(journal_text: str) -> tuple[bool, str]:
    """`(ok, message)` for the dogfood's AI stages in `journal_text`."""
    # `DOGFOOD OK` is a substring of nothing else here, but `DOGFOOD ASK ok` would
    # match a hypothetical `DOGFOOD ASK ok=false`, so the stage checks stay exact
    # strings on their own lines rather than loose `in journal` tests.
    lines = [l.strip() for l in journal_text.splitlines()]
    emitted = any(l.endswith(EMIT_OK) or EMIT_OK + " " in l for l in lines)
    asked = any(ASK_OK in l for l in lines)
    finished = any(l.endswith(TERMINAL_OK) for l in lines)

    if not finished:
        fail = _quoted(journal_text, FAIL_MARKER)
        return False, (
            "the in-VM KG-AI dogfood did not complete"
            + (f":{fail}" if fail else " (no DOGFOOD OK marker)")
        )
    if not emitted:
        return False, (
            "the dogfood finished without injecting an event: no "
            f"'{EMIT_OK}'. --require-dogfood asserts an event reached the bus, and "
            "a completion about no event is not evidence for this boot."
        )
    if not asked:
        skip = _quoted(journal_text, ASK_SKIPPED)
        return False, (
            "the dogfood finished without an AI completion: no "
            f"'{ASK_OK}'. The terminal marker is printed whatever happened - the "
            "ask is best-effort by design - so a run that answered nothing "
            "reaches here looking finished." + (f" The probe's reason:{skip}" if skip else "")
        )
    return True, "the dogfood injected an event and got a completion back"


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(__doc__.strip().splitlines()[0], file=sys.stderr)
        print(f"usage: {argv[0]} <journal-or-serial-log>", file=sys.stderr)
        return 2
    try:
        with open(argv[1], "r", errors="replace") as fh:
            text = fh.read()
    except OSError as e:
        print(f"could not read {argv[1]}: {e}", file=sys.stderr)
        return 2
    ok, message = ai_verdict(text)
    print(("ai: " if ok else "VERIFY FAIL: ") + message)
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
