#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""What each daemon puts on the bus when nothing is happening.

A producer that emits on a timer needs a guard saying what counts as a change,
and three of those guards were wrong in one night, each in a way no test could
see: the journald parser turned a bluetoothd error into a device transition, the
same parser reported a device going down every time an already-down device
changed which flavour of down it was, and the power daemon republished the whole
power state every nine seconds because a running time estimate is part of the
struct it compared.

All three looked correct in review and all three were found the same way - start
it, touch nothing, count. This does that on purpose instead of by luck.

WHAT IT MEASURES. One daemon at a time, each in its own throwaway runtime
directory with a private event bus and no peers, watched for a fixed window while
the machine is left alone. The number reported is events per window per topic.

HOW TO READ IT. **A non-zero row is not automatically a defect.** A daemon whose
subject genuinely changes while you watch - a laptop crossing a battery
threshold, a journal that is genuinely busy - is right to emit. What the number
answers is "is this daemon's idea of a change the same as mine", and a steady
stream against an idle machine is the question worth asking.

IT DOES NOT PRE-CLASSIFY WHO PRODUCES. Every runnable daemon is started and the
bus is watched; a daemon that emits nothing is a zero row rather than an absence.
Deriving the producer set first would mean a third hand-kept list beside
`check-emitters-declared.py`'s and the smoke's, and this directory's own history
says those drift - the smoke's header is an essay about exactly that.

THE ROSTER COMES FROM `smoke-daemons.sh`, parsed, because it already records
which daemons can run unattended AND the environment each needs to keep its state
disposable. A daemon in that file's SKIPPED list is skipped here for the same
reason, which is printed.

This is a sweep, not a gate: a full run is minutes, the answer needs a person to
read it, and a busy machine legitimately moves the numbers. It is not in CI.
"""

import argparse
import os
import re
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SMOKE = REPO / "dev/scripts/smoke-daemons.sh"
BUS = "event-bus"

#: `"arlen-auditd|audit-ingest.sock|"` inside the DAEMONS array.
ENTRY = re.compile(r'^\s*"(?P<name>[a-z0-9-]+)\|(?P<sock>[^|]*)\|(?P<env>[^"]*)"', re.M)
#: `"arlen-powerd|needs the system bus (UPower, logind)"` inside SKIPPED.
SKIP_ENTRY = re.compile(r'^\s*"(?P<name>[a-z0-9-]+)\|(?P<reason>[^"]+)"', re.M)


def roster() -> tuple[list[tuple[str, str]], list[tuple[str, str]]]:
    """(name, env) for every runnable daemon, and (name, reason) for the skipped."""
    text = SMOKE.read_text(encoding="utf-8")
    run_block = text.split("DAEMONS=(", 1)[1].split("\n)", 1)[0]
    skip_block = text.split("SKIPPED=(", 1)[1].split("\n)", 1)[0]
    runnable = [(m.group("name"), m.group("env")) for m in ENTRY.finditer(run_block)]
    skipped = [(m.group("name"), m.group("reason")) for m in SKIP_ENTRY.finditer(skip_block)]
    return runnable, skipped


def spawn(
    binary: Path,
    rt: Path,
    extra: str,
    log: Path,
    args: list[str] | None = None,
) -> subprocess.Popen | None:
    """Start `binary` with its state inside `rt`. `$rt` in `extra` is this run's dir."""
    env = dict(os.environ)
    env.update(
        {
            "XDG_RUNTIME_DIR": str(rt),
            "XDG_DATA_HOME": str(rt / "data"),
            "XDG_STATE_HOME": str(rt / "state"),
            "XDG_CONFIG_HOME": str(rt / "config"),
            "ARLEN_RUNTIME_DIR": str(rt),
            "ARLEN_SESSION_ID": "idle-census",
        }
    )
    # The smoke stores `\$rt` escaped, because bash resolves it with its own
    # `eval` against that run's directory. Substituting only the unescaped form
    # leaves a literal backslash in the path, which SQLite reports as "unable to
    # open database file" - read as the daemon failing to start, when it was the
    # env this file handed it.
    for pair in extra.replace("\\$rt", str(rt)).replace("$rt", str(rt)).split():
        if "=" in pair:
            k, v = pair.split("=", 1)
            env[k] = v
    try:
        return subprocess.Popen(
            [str(binary), *(args or [])],
            env=env,
            stdout=log.open("wb"),
            stderr=subprocess.STDOUT,
            stdin=subprocess.DEVNULL,
            start_new_session=True,
        )
    except OSError as e:
        print(f"     could not start: {e}", file=sys.stderr)
        return None


def count_events(consumer_sock: Path, window: float) -> dict[str, int]:
    """Topic -> events seen in `window` seconds on a bus nobody else is using."""
    seen: dict[str, int] = {}
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(1.0)
    try:
        s.connect(str(consumer_sock))
        # id, prefixes, uid filter - the three lines the bus reads.
        s.sendall(b"idle-census\n*\n*\n")
    except OSError as e:
        print(f"     bus consumer refused: {e}", file=sys.stderr)
        return seen
    end = time.monotonic() + window
    while time.monotonic() < end:
        try:
            head = s.recv(4)
            if len(head) < 4:
                break
            n = struct.unpack(">I", head)[0]
            body = b""
            while len(body) < n:
                chunk = s.recv(n - len(body))
                if not chunk:
                    break
                body += chunk
            # The topic is field 2 of the envelope, a length-delimited string
            # right after the id. Read it positionally rather than importing a
            # protobuf runtime into a sweep script.
            topic = "?"
            if len(body) > 2 and body[0] == 0x0A:
                after_id = 2 + body[1]
                if len(body) > after_id + 1 and body[after_id] == 0x12:
                    ln = body[after_id + 1]
                    topic = body[after_id + 2 : after_id + 2 + ln].decode("utf-8", "replace")
            seen[topic] = seen.get(topic, 0) + 1
        except socket.timeout:
            continue
        except OSError:
            break
    s.close()
    return seen


def stop(proc: subprocess.Popen | None) -> None:
    if proc is None or proc.poll() is not None:
        return
    try:
        os.killpg(proc.pid, signal.SIGTERM)
    except OSError:
        proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except OSError:
            proc.kill()


def counter_is_working(target: Path, window: float) -> bool:
    """Prove the counter can count before believing a page of zeros.

    A census whose every row reads `silent` is indistinguishable from a census
    whose consumer never connected, and the first full run of this file produced
    exactly that page. So one known event goes through the same path - private
    bus, same subscribe, same frame reader - and the run refuses to report
    anything if it does not come back.

    `arlen-event-emit` is the tree's own injector and already waits to see its
    own event return, so a failure here is this file's counter and not the bus.
    """
    emitter = target / "arlen-event-emit"
    if not emitter.is_file():
        print("cannot self-test: arlen-event-emit is not built", file=sys.stderr)
        return False
    rt = Path(tempfile.mkdtemp(prefix="arlen-census-selftest."))
    rt.chmod(0o700)
    (rt / "arlen").mkdir(parents=True, exist_ok=True)
    bus = spawn(target / BUS, rt, "", rt / "bus.log")
    consumer = rt / "arlen/event-bus-consumer.sock"
    for _ in range(20):
        if consumer.is_socket():
            break
        time.sleep(0.25)

    seen: dict[str, int] = {}

    def watch() -> None:
        seen.update(count_events(consumer, min(window, 8.0)))

    import threading

    t = threading.Thread(target=watch, daemon=True)
    t.start()
    time.sleep(1.0)
    # It takes a path and refuses without one: called bare it prints usage and
    # exits, which is how the first cut of this self-test "proved" the counter
    # broken when the counter was fine.
    proc = spawn(emitter, rt, "", rt / "emit.log", ["/work/census/probe.txt", "census"])
    if proc is not None:
        try:
            proc.wait(timeout=15)
        except subprocess.TimeoutExpired:
            stop(proc)
    t.join(timeout=min(window, 8.0) + 3)

    stop(bus)
    total = sum(seen.values())
    shutil.rmtree(rt, ignore_errors=True)
    if total:
        print(f"counter self-test: {total} event(s) counted, the reader works\n")
        return True
    print("counter self-test: counted NOTHING through a known emit", file=sys.stderr)
    return False


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--window", type=float, default=20.0, help="seconds to watch each daemon")
    ap.add_argument("--only", default="", help="run just this daemon")
    args = ap.parse_args()

    target = REPO / "target/debug"
    bus_bin = target / BUS
    if not bus_bin.is_file():
        print(f"the event bus is not built at {bus_bin}", file=sys.stderr)
        return 2

    runnable, skipped = roster()
    if not runnable:
        print(f"NOTHING WAS READ: no daemon parsed out of {SMOKE}", file=sys.stderr)
        return 2
    if args.only:
        runnable = [r for r in runnable if r[0] == args.only]
        if not runnable:
            print(f"no runnable daemon named {args.only}", file=sys.stderr)
            return 2

    if not counter_is_working(target, args.window):
        print(
            "refusing to report: a page of zeros from a reader that cannot count is\n"
            "worse than no census, and that is what the first run of this produced.",
            file=sys.stderr,
        )
        return 2

    print(f"idle emitter census: {args.window:g}s per daemon, machine left alone\n")
    rows: list[tuple[str, int, str]] = []
    unbuilt: list[str] = []

    for name, extra in runnable:
        if name == BUS:
            continue  # it is the bus, not a producer onto it
        binary = target / name
        if not binary.is_file():
            unbuilt.append(name)
            continue

        rt = Path(tempfile.mkdtemp(prefix="arlen-census."))
        rt.chmod(0o700)
        for sub in ("arlen", "knowledge", "vault", "data", "state", "config"):
            (rt / sub).mkdir(parents=True, exist_ok=True)

        bus = spawn(bus_bin, rt, "", rt / "bus.log")
        consumer = rt / "arlen/event-bus-consumer.sock"
        for _ in range(20):
            if consumer.is_socket():
                break
            time.sleep(0.25)

        proc = spawn(binary, rt, extra, rt / "log")
        # A moment to connect as a producer before the window opens, so a first
        # emit is not lost to a race and counted as silence.
        time.sleep(2.0)
        alive = proc is not None and proc.poll() is None
        topics = count_events(consumer, args.window) if alive else {}

        stop(proc)
        stop(bus)
        shutil.rmtree(rt, ignore_errors=True)

        total = sum(topics.values())
        if not alive:
            rows.append((name, -1, "did not stay up"))
            print(f"  --   {name:26} did not stay up")
            continue
        detail = ", ".join(f"{t} x{c}" for t, c in sorted(topics.items())) or "silent"
        rows.append((name, total, detail))
        mark = "**" if total else "ok"
        print(f"  {mark:4} {name:26} {total:4} event(s)  {detail}")

    print()
    noisy = [(n, c, d) for n, c, d in rows if c > 0]
    if noisy:
        print(f"{len(noisy)} daemon(s) emitted with nothing happening:\n")
        for n, c, d in sorted(noisy, key=lambda r: -r[1]):
            print(f"  {n}: {c} in {args.window:g}s - {d}")
        print(
            "\nNot automatically a defect: a subject that genuinely changed while you\n"
            "watched is worth an event. Read each one and decide whether the daemon's\n"
            "idea of a change is the same as yours."
        )
    else:
        print("every daemon watched was silent with nothing happening")

    if unbuilt:
        print(f"\nnot built, so not measured: {', '.join(sorted(unbuilt))}")
    print(f"\n{len(skipped)} daemon(s) cannot run unattended (from the smoke's own list):")
    for n, reason in sorted(skipped):
        print(f"  - {n}: {reason}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
