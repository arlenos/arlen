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
import pathlib
import re
import sqlite3
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ingest_verdict import DOGFOOD_PATH, INGEST_TYPE, ingest_verdict

REPO = pathlib.Path(__file__).resolve().parents[2]
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

    # The lag half. The verify run keeps ending on "promotion had not reached this
    # event", which is true and unactionable without a size, so the size is now in
    # the sentence. These build a store WITH a high-water mark and check the
    # arithmetic rather than trusting it.
    with tempfile.TemporaryDirectory() as d:
        from ingest_verdict import PROMOTION_BATCH, PROMOTION_INTERVAL_S, promotion_lag

        # 2500 events stamped after the mark: three passes of 1000, so 90s.
        db = os.path.join(d, "behind.db")
        conn = sqlite3.connect(db)
        conn.execute(
            "CREATE TABLE events (id TEXT PRIMARY KEY, type TEXT NOT NULL, "
            "timestamp INTEGER NOT NULL, source TEXT NOT NULL, pid INTEGER NOT NULL, "
            "origin TEXT NOT NULL, payload BLOB)"
        )
        conn.execute("CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT)")
        conn.execute("INSERT INTO metadata VALUES ('promotion_hwm', '100')")
        for i in range(2500):
            conn.execute(
                "INSERT INTO events VALUES (?, 'file.opened', ?, 'test', 1, 'system:test', ?)",
                (f"b{i}", 200, b""),
            )
        conn.commit()
        conn.close()
        lag = promotion_lag(db)
        check("a backlog is reported with its size", "2500 event(s) behind" in lag, True)
        check("and converted into how long it takes to clear", "1m30s" in lag, True)

        # Nothing after the mark is the state a passing run should describe.
        db2 = os.path.join(d, "caught-up.db")
        conn = sqlite3.connect(db2)
        conn.execute(
            "CREATE TABLE events (id TEXT PRIMARY KEY, type TEXT NOT NULL, "
            "timestamp INTEGER NOT NULL, source TEXT NOT NULL, pid INTEGER NOT NULL, "
            "origin TEXT NOT NULL, payload BLOB)"
        )
        conn.execute("CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT)")
        conn.execute("INSERT INTO metadata VALUES ('promotion_hwm', '999')")
        conn.execute(
            "INSERT INTO events VALUES ('a', 'file.opened', 5, 'test', 1, 'system:test', x'')"
        )
        conn.commit()
        conn.close()
        check("a caught-up store says so", "caught up" in promotion_lag(db2), True)

        # A store with no metadata table at all must not fail the run: the lag is
        # an aside, and an aside that can refuse is a new way to go red.
        db3 = make_store(os.path.join(d, "nometa.db"), [("file.opened", b"x")])
        check("a store without the mark stays quiet", promotion_lag(db3), "")

        # The two constants are a COPY of the daemon's, which is how a number in
        # one language quietly stops describing the other. Read both out of the
        # Rust and compare, so the lag sentence cannot go on quoting a batch size
        # nobody uses.
        promo = (REPO / "daemons/knowledge/src/promotion.rs").read_text()
        graph = (REPO / "sdk/os-sdk/src/graph.rs").read_text()
        batch = re.search(r"const PROMOTION_BATCH: i64 = (\d+)", promo)
        interval = re.search(r"PROMOTION_INTERVAL: std::time::Duration = std::time::Duration::from_secs\((\d+)\)", graph)
        check("the batch size here is the daemon's", int(batch.group(1)) if batch else None, PROMOTION_BATCH)
        check(
            "and so is the interval",
            int(interval.group(1)) if interval else None,
            PROMOTION_INTERVAL_S,
        )

    if FAILURES:
        print(f"\n{len(FAILURES)} check(s) failed: {', '.join(FAILURES)}")
        return 1
    print("\nall ingestion checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
