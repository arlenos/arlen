#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Ask the guest's event store directly whether this boot's file arrived.

Every other boot assertion we have reads what a process SAID about itself. The
knowledge probe queries the graph and prints its findings; `probe_verdict` then
grades those printed lines. That chain has a hole at the top: if the probe asks
the wrong question, miscounts its rows, or prints a number it did not measure,
the verdict agrees with it. A self-report cannot catch a component that reports
wrongly about itself, and the probe is a component.

So this check never speaks to the probe. It takes the guest's own SQLite event
store, copied out of the halted disk image, and runs the question in SQL on the
host. Nothing in the guest gets a vote in the answer.

The question is deliberately the narrow one: did a `file.opened` for the path
this boot emits reach the store? That is the first hop of the ingestion path -
emitter to writer to disk - and it is the hop where a byte-exact external answer
is available. The rest of the path (promotion turning that row into a File node)
stays with the probe, because the graph store is not externally readable in a way
that could carry a verdict: a byte search of it finds the schema strings and not
the values, so a miss there means either "not stored verbatim" or "never written"
and the check could not tell you which. A gate whose failure has two meanings is
worse than no gate.

Run it against a copied-out store:

    dev/vm/ingest_verdict.py /tmp/events.db
"""

import os
import sqlite3
import sys

# The path the dogfood emits and the probe asks about, named here so the SQL can
# be byte-exact about it. Hardcoded in three places now - the emitter, the probe's
# query, the check below - and the control test asserts this string still appears
# in `dev/kg-probe`, so the coupling breaks loudly at test time rather than
# quietly at boot time.
DOGFOOD_PATH = "/var/lib/arlen-work/notes.md"

# The event the emitter sends for that path.
INGEST_TYPE = "file.opened"


def _query(db_path: str, sql: str, params: tuple = ()) -> list:
    """Run `sql` read-only against `db_path`."""
    # `mode=ro` on a URI so a half-written store cannot be modified by reading it,
    # and so a missing file raises here rather than being created empty - sqlite3
    # will happily conjure a new database at a path that does not exist, which
    # would turn "the guest never wrote a store" into "the store is empty" and
    # lose the distinction between two different failures.
    uri = f"file:{db_path}?mode=ro"
    with sqlite3.connect(uri, uri=True) as conn:
        return conn.execute(sql, params).fetchall()


def ingest_verdict(db_path: str) -> tuple[bool, str]:
    """`(ok, message)` for the guest's event store at `db_path`."""
    if not os.path.exists(db_path):
        return False, (
            "the guest's event store was not found. The knowledge daemon creates "
            "it on first write, so a missing file means the daemon never wrote a "
            "single event - not that this run's file is missing from it."
        )

    try:
        tables = _query(
            db_path, "SELECT name FROM sqlite_master WHERE type='table' AND name='events'"
        )
    except sqlite3.Error as e:
        return False, f"the guest's event store could not be read: {e}"

    if not tables:
        return False, (
            "the guest's event store has no `events` table. The file exists but "
            "the schema was never created, so the daemon opened it and got no "
            "further."
        )

    try:
        total = _query(db_path, "SELECT count(*) FROM events")[0][0]
        # `hex()` on both sides rather than a LIKE against the blob: the payload is
        # protobuf, so it carries NUL bytes, and SQLite's string functions stop at
        # the first one. Comparing hex is byte-exact and has no such edge - checked
        # against a real store, where it returned 5 for a path present and 0 for a
        # path absent.
        mine = _query(
            db_path,
            "SELECT count(*) FROM events WHERE type = ? "
            "AND hex(payload) LIKE '%' || hex(?) || '%'",
            (INGEST_TYPE, DOGFOOD_PATH),
        )[0][0]
    except sqlite3.Error as e:
        return False, f"the guest's event store could not be queried: {e}"

    if total == 0:
        return False, (
            "the guest's event store is empty. The desktop came up and the writer "
            "stored nothing at all, so the ingestion path is not slow, it is not "
            "running."
        )

    if mine == 0:
        # The refusal this exists for, and the one a row count cannot make. A boot
        # that emits nothing of its own still fills the store with menu
        # registrations and project scans, so "the store has rows" passes on
        # traffic that says nothing about whether the emit-to-disk path worked.
        return False, (
            f"the guest's event store holds {total} event(s), but none of them is "
            f"this run's file: no `{INGEST_TYPE}` payload contains "
            f"`{DOGFOOD_PATH}`. Something was written, so the writer is alive; "
            f"what the boot emitted did not reach it."
        )

    return True, (
        f"the guest's event store holds this run's file: {mine} `{INGEST_TYPE}` "
        f"row(s) for `{DOGFOOD_PATH}`, out of {total} event(s), read from the "
        f"store itself rather than from anything the guest said about it"
    )


def main() -> int:
    if len(sys.argv) != 2:
        print(f"usage: {sys.argv[0]} <events.db>", file=sys.stderr)
        return 2
    ok, message = ingest_verdict(sys.argv[1])
    print(("event store: " if ok else "VERIFY FAIL: ") + message)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
