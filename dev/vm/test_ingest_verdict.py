#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Show the external ingestion check refusing, on stores built to be wrong.

`ingest_verdict` reads the guest's event store instead of grading what the guest
said about itself, which removes one way to be fooled and adds another: a check
nobody has watched refuse is indistinguishable from `return True`, and this one
runs on boots where the store is healthy, so its refusals would never fire on
their own.

Each store below is built to carry exactly one defect - missing, unschema'd,
empty, full of other traffic - and the check is asserted to refuse it. The last
one is the case worth the file: a store with plenty of rows and none of them from
this boot, which is what a broken emit path looks like from outside and what a
row count would wave through.

Run: python3 dev/vm/test_ingest_verdict.py
"""

import os
import sqlite3
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ingest_verdict import DOGFOOD_PATH, INGEST_TYPE, ingest_verdict

FAILURES = []


def check(name: str, got, want) -> None:
    ok = got == want
    print(f"  {'ok  ' if ok else 'FAIL'}  {name}")
    if not ok:
        print(f"        wanted {want!r}, got {got!r}")
        FAILURES.append(name)


def make_store(path: str, rows: list, schema: bool = True) -> str:
    """A store at `path` holding `rows` as `(type, payload_bytes)`."""
    conn = sqlite3.connect(path)
    if schema:
        conn.execute(
            "CREATE TABLE events (id TEXT PRIMARY KEY, type TEXT NOT NULL, "
            "timestamp INTEGER NOT NULL, source TEXT NOT NULL, pid INTEGER NOT NULL, "
            "origin TEXT NOT NULL, payload BLOB)"
        )
        for i, (etype, payload) in enumerate(rows):
            conn.execute(
                "INSERT INTO events VALUES (?, ?, ?, ?, ?, ?, ?)",
                (f"e{i}", etype, 1, "test", 1, "system:test", payload),
            )
    else:
        # A file that is a database but has never had the schema applied, which is
        # what an open-then-die looks like on disk.
        conn.execute("CREATE TABLE unrelated (x INTEGER)")
    conn.commit()
    conn.close()
    return path


def payload_for(path: str) -> bytes:
    """A payload shaped like the real one: protobuf framing around a path.

    The NUL and the 0x12 field tag are the point. The real payload is protobuf, so
    it carries bytes that terminate a C string, and an earlier draft of the check
    used a plain LIKE against the blob - which stops at the first NUL and would
    have missed a path stored after one.
    """
    return b"\x0a" + bytes([len(path)]) + path.encode() + b"\x00\x12\x07dogfood"


def main() -> int:
    print("external ingestion check")
    with tempfile.TemporaryDirectory() as tmp:
        missing = os.path.join(tmp, "nope.db")
        ok, msg = ingest_verdict(missing)
        check("a store that was never written is refused", ok, False)
        check(
            "and it says the daemon wrote nothing, not that the file is empty",
            "never wrote a single event" in msg,
            True,
        )
        check(
            "reading a missing store does not create one",
            os.path.exists(missing),
            False,
        )

        no_schema = make_store(os.path.join(tmp, "noschema.db"), [], schema=False)
        check("a store with no events table is refused", ingest_verdict(no_schema)[0], False)

        empty = make_store(os.path.join(tmp, "empty.db"), [])
        ok, msg = ingest_verdict(empty)
        check("an empty store is refused", ok, False)
        check("and it says so plainly", "is empty" in msg, True)

        # The one this file exists for. Thirty rows of ordinary session traffic and
        # a file.opened for a DIFFERENT file: every count is healthy, the writer is
        # demonstrably alive, and the thing the boot emitted never arrived.
        others = [("app.menu.registered", b"\x0a\x04menu")] * 30
        others.append((INGEST_TYPE, payload_for("/home/someone/unrelated.md")))
        busy = make_store(os.path.join(tmp, "busy.db"), others)
        ok, msg = ingest_verdict(busy)
        check("a store full of other traffic is refused", ok, False)
        check("and it reports the rows it did find", "31 event(s)" in msg, True)

        # A file.opened carrying the path, but under the wrong event type: the
        # emitter sent something, and not the thing that drives promotion.
        wrong_type = make_store(
            os.path.join(tmp, "wrongtype.db"),
            [("file.written", payload_for(DOGFOOD_PATH))],
        )
        check(
            "the right path under the wrong event type is refused",
            ingest_verdict(wrong_type)[0],
            False,
        )

        good = make_store(
            os.path.join(tmp, "good.db"),
            [("app.menu.registered", b"\x0a\x04menu"),
             (INGEST_TYPE, payload_for(DOGFOOD_PATH))],
        )
        ok, msg = ingest_verdict(good)
        check("a store holding this run's file passes", ok, True)
        check("and it names what it found", DOGFOOD_PATH in msg, True)
        check(
            "the path is found even though a NUL byte precedes the rest",
            "1 `file.opened` row(s)" in msg,
            True,
        )

    # The coupling that would otherwise break in silence. The path is written out
    # in the emitter, in the probe's query and in this check; if one moves, the
    # boot check starts refusing healthy systems and the reason is three files
    # away. Asserting it here means the rename fails at test time instead.
    probe = os.path.join(
        os.path.dirname(os.path.abspath(__file__)), "..", "kg-probe", "src", "main.rs"
    )
    try:
        with open(probe, encoding="utf-8") as fh:
            in_probe = DOGFOOD_PATH in fh.read()
    except OSError:
        in_probe = False
    check("the probe still asks about the same path this check looks for", in_probe, True)

    if FAILURES:
        print(f"\n{len(FAILURES)} check(s) failed: {', '.join(FAILURES)}")
        return 1
    print("\nall ingestion checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
