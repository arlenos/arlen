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

So the flag asserted a completion it never checked, and it asserted an EVENT it
never checked either. The second half is the one that had to become a refusal:
a boot that injected nothing and printed the terminal marker passed. This is
deepseek's rule 1 in our own tree - the harness trusted the agent's summary of
itself instead of the thing the summary is about.

The completion half resolved the other way, and the reasoning is in `ai_verdict`
beside the branch: the ask leg is best-effort by design and is currently refused
by an admission decision nobody has made, so requiring it would paint every boot
red over an open question. The false claim in the help text is what gets fixed
there, not the gate.

The stages are separate facts and are kept separate here - the same reason their
rule 2 asks for `timedOut`, `signal` and `exitCode` side by side. An umbrella
marker that outlives its stages is not a verdict about them.

Two refusals:

    never finished   no terminal marker at all: the run died, or ended before the
                     probe did. The probe's own `DOGFOOD FAIL` line is quoted when
                     there is one, because it names the cause and this cannot.
    no event         `DOGFOOD EMIT ok` absent. The first half of what the flag
                     claims: nothing was injected, so whatever else happened was
                     not about this boot.
Not a refusal, deliberately: a missing `DOGFOOD ASK ok`. It is reported in the
message with the probe's own reason, and the reasoning for leaving it ungated is
at that branch.

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

    # A journal with no DOGFOOD line at all did not measure the dogfood - it is
    # either a probe that never started or, more often, output that never reached
    # this side. The caller reads its serial log with `except OSError: journal =
    # ""`, so an unreadable log arrives here as the empty string, and saying "the
    # dogfood did not complete" about it is a claim about the guest drawn from a
    # file this machine could not read. Same shape as the copy-out that reported
    # a failed guestfish as "the guest wrote no store".
    if not any("DOGFOOD" in l for l in lines):
        return False, (
            "the dogfood left no trace in the journal, so this run measured nothing "
            "about the AI path - check that the serial log was captured before "
            "reading this as a dogfood failure"
        )

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
    # The ask is NOT gated, and the first version of this file got that wrong.
    #
    # It refused a boot with no `DOGFOOD ASK ok`, which reads right from the flag's
    # help text and is wrong against the probe: the ask leg is best-effort by
    # design, and it is currently refused for a reason nobody has decided yet -
    # `ai1_iface.rs` gates `explain_system` on `user_surface_admitted` and the
    # dogfood is not a user surface. Gating on it would paint every boot red over
    # an open admission decision, which is manufacturing a red board rather than
    # finding one. The probe's own note says it plainly: a failure there is not a
    # dogfood failure, the executor write is the deterministic proof.
    #
    # So the claim gets corrected rather than the gate tightened - the flag no
    # longer promises a completion it deliberately does not require, and the stage
    # line beside this verdict says whether one came back.
    if not asked:
        skip = _quoted(journal_text, ASK_SKIPPED)
        return True, (
            "the dogfood injected an event and finished; the AI completion leg is "
            "best-effort and did not answer this boot." + (f" Its reason:{skip}" if skip else "")
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
