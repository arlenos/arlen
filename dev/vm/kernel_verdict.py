#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Ask the guest's event store whether the eBPF sensor produced anything.

The third of the store-reading verdicts, and the one that closes a hole the other
two cannot see. `ingest_verdict` asks whether the dogfood's `file.opened` reached
the store; `graph_verdict` asks whether it became a File node. Both are satisfied
by events the DESKTOP emits. The kernel layer is the machine-wide sensor, and on
every image built so far it was not present at all, so the question "does the
graph get file events from the kernel on a real install" had no answer anywhere.

It has a specific failure worth catching. The eBPF program is loaded at runtime
and the verifier can refuse it - it did, on the first image that carried the
sensor:

    Unreleased reference id=5 alloc_insn=11
    the BPF_PROG_LOAD syscall failed ... Invalid argument (os error 22)

That refusal is invisible from everywhere except the sensor's own journal. The
unit is `Restart=on-failure`, so it retries and keeps failing, the desktop comes
up looking perfectly well, and the graph simply has no file events in it. Nothing
in the boot says so, because nothing was asking.

The question here is the narrow one an external reader can settle: did ANY row in
the guest's event store carry the sensor's source. Not a particular path - the
sensor observes whatever the machine happens to open, so naming a file would be
asserting something about the boot's timing rather than about the sensor.

The store is read on the host, out of the halted image, so nothing in the guest
gets a vote on the answer.

Run it against a copied-out store:

    dev/vm/kernel_verdict.py /tmp/events.db
"""

import os
import sqlite3
import sys

#: What the normalizer stamps on every event it forwards
#: (`kernel-layer/src/normalizer.rs`). The one string that separates the sensor's
#: events from the desktop's in a store that holds both.
KERNEL_SOURCE = "ebpf"

#: And the session identity it uses when nothing set one, for the message: an
#: event that happens in no session still needs a named origin
#: (`kernel-layer/src/main.rs`).
KERNEL_ORIGIN = "system:kernel-layer"


def _query(db_path: str, sql: str, params: tuple = ()) -> list:
    """Run `sql` read-only against `db_path`."""
    # `mode=ro` on a URI, for the same reason as the sibling: sqlite3 will
    # conjure an empty database at a path that does not exist, which would turn
    # "the guest never wrote a store" into "the store is empty" and lose the
    # difference between two different failures.
    uri = f"file:{db_path}?mode=ro"
    with sqlite3.connect(uri, uri=True) as conn:
        return conn.execute(sql, params).fetchall()


def kernel_verdict(db_path: str) -> tuple[bool, str]:
    """`(ok, message)` for the sensor's presence in the store at `db_path`."""
    if not os.path.exists(db_path):
        return False, (
            "the guest's event store was not found. The knowledge daemon creates "
            "it on first write, so a missing file means the daemon never wrote "
            "anything - a question about the sensor cannot be answered from here."
        )

    try:
        tables = _query(
            db_path,
            "SELECT name FROM sqlite_master WHERE type='table' AND name='events'",
        )
    except sqlite3.Error as e:
        return False, f"the guest's event store could not be read: {e}"

    if not tables:
        return False, (
            "the guest's event store has no `events` table, so the daemon opened "
            "it and got no further. Nothing about the sensor follows from that."
        )

    try:
        total = _query(db_path, "SELECT count(*) FROM events")[0][0]
        mine = _query(
            db_path, "SELECT count(*) FROM events WHERE source = ?", (KERNEL_SOURCE,)
        )[0][0]
    except sqlite3.Error as e:
        return False, f"the guest's event store could not be queried: {e}"

    if total == 0:
        return False, (
            "the guest's event store is empty, so the writer stored nothing at "
            "all. That is a broader failure than the sensor and this check is not "
            "the one that names it."
        )

    if mine == 0:
        # The refusal this file exists for. Every other row in the store came from
        # the desktop, so a store full of rows says nothing about the sensor, and
        # a boot that never loaded the BPF program looks exactly like a healthy one
        # from every other angle.
        return False, (
            f"{total} event(s) in the store and not one from the kernel sensor "
            f"(`source = {KERNEL_SOURCE!r}`). Either the unit did not run, or the "
            f"verifier refused the BPF program - which it does silently as far as "
            f"anything outside the sensor's own journal is concerned, while the "
            f"unit restarts and the desktop comes up looking well. The graph's "
            f"file picture on this boot came from the desktop alone."
        )

    # WHICH types, not just how many. The first boot this ran against forwarded 50
    # events and every one was `process.started`: the sensor had loaded and its
    # file probe had produced nothing, and a bare count called that a working
    # sensor. Naming the types puts the difference in the line rather than leaving
    # it for whoever thinks to open the store afterwards.
    try:
        kinds = _query(
            db_path,
            "SELECT type, count(*) FROM events WHERE source = ? "
            "GROUP BY type ORDER BY count(*) DESC",
            (KERNEL_SOURCE,),
        )
    except sqlite3.Error:
        kinds = []
    breakdown = ", ".join(f"{n}x {t}" for t, n in kinds) if kinds else "types unreadable"

    return True, (
        f"{mine} of {total} event(s) came from the kernel sensor "
        f"(`source = {KERNEL_SOURCE!r}`, origin `{KERNEL_ORIGIN}`), so the BPF "
        f"program loaded, attached and forwarded on this boot: {breakdown}."
    )


def main() -> int:
    if len(sys.argv) != 2:
        print(f"usage: {sys.argv[0]} <events.db copied out of the guest>", file=sys.stderr)
        return 2
    ok, message = kernel_verdict(sys.argv[1])
    print(f"KERNEL {'ok' if ok else 'FAILED'}: {message}")
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
