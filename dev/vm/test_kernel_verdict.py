#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Show the kernel-sensor check refusing, on stores built to be wrong.

Same argument as `test_ingest_verdict`: a check nobody has watched refuse is
indistinguishable from `return True`, and this one is meant to run on boots where
the store is otherwise healthy, so its refusals would never fire on their own.

The case worth the file is the last one - a store with plenty of rows and not one
of them from the sensor. That is exactly what a boot looks like when the BPF
verifier refuses the program: the unit restarts quietly, the desktop comes up, the
store fills with desktop traffic, and the machine-wide file picture is simply
absent. A row count waves it through; so does every screenshot.

Run: python3 dev/vm/test_kernel_verdict.py
"""

import os
import sqlite3
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from kernel_verdict import KERNEL_SOURCE, kernel_verdict  # noqa: E402

FAILURES = []


def check(name: str, got, want) -> None:
    ok = got == want
    print(f"  {'ok  ' if ok else 'FAIL'}  {name}")
    if not ok:
        print(f"        wanted {want!r}, got {got!r}")
        FAILURES.append(name)


def make_store(path: str, sources: list, schema: bool = True) -> str:
    """A store at `path` holding one row per entry in `sources`."""
    conn = sqlite3.connect(path)
    if schema:
        conn.execute(
            "CREATE TABLE events (id TEXT PRIMARY KEY, type TEXT NOT NULL, "
            "timestamp INTEGER NOT NULL, source TEXT NOT NULL, pid INTEGER NOT NULL, "
            "origin TEXT NOT NULL, payload BLOB)"
        )
        for i, source in enumerate(sources):
            conn.execute(
                "INSERT INTO events VALUES (?, ?, ?, ?, ?, ?, ?)",
                (f"e{i}", "file.opened", 1, source, 1, "system:test", b""),
            )
    else:
        conn.execute("CREATE TABLE unrelated (x INTEGER)")
    conn.commit()
    conn.close()
    return path


def main() -> int:
    print("kernel sensor check")
    with tempfile.TemporaryDirectory() as tmp:
        missing = os.path.join(tmp, "nope.db")
        ok, msg = kernel_verdict(missing)
        check("a store that was never written is refused", ok, False)
        check(
            "reading a missing store does not create one",
            os.path.exists(missing),
            False,
        )

        no_schema = make_store(os.path.join(tmp, "noschema.db"), [], schema=False)
        check("a store with no events table is refused", kernel_verdict(no_schema)[0], False)

        empty = make_store(os.path.join(tmp, "empty.db"), [])
        ok, msg = kernel_verdict(empty)
        check("an empty store is refused", ok, False)
        check(
            "and it says the failure is broader than the sensor",
            "broader failure" in msg,
            True,
        )

        # The one that matters. A perfectly busy desktop, no sensor.
        desktop_only = make_store(
            os.path.join(tmp, "desktop.db"),
            ["desktop-shell", "knowledge", "arlen-files", "desktop-shell"],
        )
        ok, msg = kernel_verdict(desktop_only)
        check("a store full of desktop traffic and no sensor is refused", ok, False)
        check(
            "and it names the BPF verifier as the likely cause",
            "verifier refused" in msg,
            True,
        )

        with_sensor = make_store(
            os.path.join(tmp, "sensor.db"),
            ["desktop-shell", KERNEL_SOURCE, "knowledge"],
        )
        ok, msg = kernel_verdict(with_sensor)
        check("one sensor row among desktop rows passes", ok, True)
        check("and it counts them rather than saying 'ok'", "1 of 3" in msg, True)

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) did not behave: {', '.join(FAILURES)}")
        return 1
    print("a boot whose sensor never loaded cannot pass for one whose sensor did")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
