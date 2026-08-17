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

Four refusals, and the last two are the ones this exists for:

    no tally       the probe asks twice, 75s apart, and a run that ends before
                   the second round has no verdict at all. "No verdict" must
                   never read as a pass; the natural reading of a silent probe
                   is that nothing went wrong.
    failures       it reported questions it could not ask. Its own count.
    no rows        every question was ANSWERED and every answer was empty. This
                   is the false green: authorised, asked, and the graph held
                   nothing - exactly what a broken ingestion path looks like from
                   outside, and it would otherwise pass with `0 failed`.
    not this run   rows came back, but none for the file this boot emitted. "The
                   graph holds files" and "the graph ingested what just happened"
                   are different sentences, and a boot is only evidence for the
                   second.

Run directly against a rendered journal to see the verdict for a captured boot:

    dev/vm/probe_verdict.py /tmp/boot-journal.log
"""

import re
import sys

# The probe's own gap between rounds, quoted in the failure message so a run that
# lingered too little says what to change rather than only that it failed.
ROUND_GAP_SECONDS = 75

# The probe question that names an artefact of THIS run, so its answer cannot be
# satisfied by a row from anywhere else. Kept in step with `dev/kg-probe`.
INGESTION_QUESTION = "ingestion: this run's file"


# systemd's own console line for the probe unit. It lands around 4.5s, before the
# serial stops carrying userspace output, so it is readable for free and does not
# depend on the probe having said anything afterwards.
PROBE_UNIT = "arlen-kg-probe.service"


def probe_is_shipped(serial_text: str) -> bool:
    """Whether this image ships the knowledge probe, so its verdict applies.

    The verdict below was reachable only behind `--require-probe`, which nothing
    passed - not CI, not the boot recipe, only the README. So it had a control, it
    could fail, and it was armed on no run. Deciding from the artefact instead of
    from a flag closes that: a release image has no such unit and is not held to a
    probe it does not ship, while a verify image whose probe started and then went
    quiet is refused rather than passed over.

    Keyed on the unit STARTING, not on probe output, so a probe that produced
    nothing still arms the refusal - which is the whole case the flag would have
    let through.
    """
    return PROBE_UNIT in serial_text


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
    # Checked BEFORE the empty-graph verdict, because the probe now has a third
    # state and the empty-graph message is a false statement about it. Since the
    # per-user move the daemon names its callers from the identity broker, and the
    # This branch used to fire on the probe's own "identity: NOT RESOLVED" line and
    # hand back a decision: put the probe in the shipped stamped-unit table, or
    # drop the graph coverage. Both halves of that were wrong. The probe inferred
    # the unresolved identity from an empty `access_grants`, and empty grants mean
    # no Grant node has been written yet, not that the caller has no name - the
    # binary route resolves /usr/bin/arlen-kg-probe to `kg-probe` when asked
    # directly. So the verdict escalated an inference into a fork for Tim.
    #
    # The probe now always asks its questions and prints what came back. A refused
    # question is a result, and it is caught below with the rest of them.
    if not rows:
        return False, (
            "the knowledge probe was answered but the graph was empty - every "
            "question returned 0 rows. An allowed question with no data is what a "
            "broken ingestion path looks like from here."
        )
    # The refusal the other three cannot make: rows EXIST but none of them is the
    # thing this run produced. "The graph holds files" passes on any row from any
    # source; only a row for the path the run emitted says the ingestion path
    # carried something end to end during this boot. Without it the verdict is a
    # count, and a count is what let a working-looking probe report health about a
    # graph nobody had watched fill.
    ingested = [
        l for l in lines
        if INGESTION_QUESTION in l and re.search(r": [1-9][0-9]* row\(s\)", l)
    ]
    if not ingested:
        return False, (
            f"the graph answered with rows, but nothing this run produced reached "
            f"it: the '{INGESTION_QUESTION}' question came back empty. The boot "
            f"emitted a file.opened for that path, so an empty answer means the "
            f"writer or the promotion pass did not carry it - the desktop came up "
            f"and did not do its job."
        )
    # The corroboration, and the only line here that does not come from the graph
    # daemon. Everything above is the daemon answering about itself: it says the
    # File node exists, and nothing has asked whether the event that should have
    # produced it ever reached the store. Those are two different claims, and a
    # writer that dropped the event while the graph held an older node would pass
    # every check above.
    #
    # Read as: the store line is REQUIRED once the graph has claimed the file. A
    # missing line is a probe too old to have looked; an unreadable store or a
    # zero count is a disagreement between two independent readers, which is a
    # louder finding than either could make alone.
    store = [l for l in lines if "store:" in l]
    if not store:
        return False, (
            "the graph claimed this run's file but nothing read the event store. "
            "That claim rests on the daemon answering about itself; the probe on "
            "this image is too old to corroborate it."
        )
    last = store[-1]
    if "UNREADABLE" in last:
        return False, (
            f"the graph claimed this run's file and the event store could not be "
            f"read to confirm it: {last.strip()}"
        )
    if not re.search(r"store: [1-9][0-9]* event row\(s\)", last):
        return False, (
            "the graph holds a File node for this run's path, but the event store "
            "holds no event naming it. Two readers disagree: the node cannot have "
            "come from this boot's ingestion, so either the writer dropped the "
            "event or the node predates the run."
        )
    return True, (
        f"{tally[-1].split('kg-probe:')[-1].strip()}, "
        f"{len(rows)} question(s) returned rows, including this run's own file, "
        f"corroborated by the event store"
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
