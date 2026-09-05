#!/usr/bin/env python3
"""Fixtures for the boot driver's timed schedule.

`--shot-at`, `--click-at` and `--key-at` share one clock, and that sharing is the
only reason a finding like the consent-gap one is evidence rather than a story: a
click has to land BETWEEN two frames, and the frame captured at the click is what
says what was on screen when the input was taken. If the schedule ran out of order,
or dropped a mistyped spec, the run would still finish and still print a verdict.

So the ordering is tested here without a VM, which is where it can be tested at
all.

Run: python3 dev/vm/test_timed_plan.py
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import json  # noqa: E402

from verify import QmpError, qmp, timed_plan  # noqa: E402

failures = []


def check(name, ok):
    print(f"  {'ok  ' if ok else 'FAIL'} {name}")
    if not ok:
        failures.append(name)


plan = timed_plan([16, 13], ["14.2:795,490"], ["20:esc"])
check("everything is in time order", [t for t, _, _ in plan] == [13, 14.2, 16, 20])
check(
    "the click carries its coordinates",
    plan[1] == (14.2, "click", (795, 490)),
)
check("the key carries its name", plan[3] == (20.0, "key", "esc"))

# A shot and a click at the same instant: the shot goes first, so the frame says
# what the click was aimed at rather than what it produced.
same = timed_plan([5], ["5:1,2"], None)
check("a shot at the same second comes before the click", [k for _, k, _ in same] == ["shot", "click"])

for bad in ("14.2", "14.2:795", ":795,490"):
    try:
        timed_plan(None, [bad], None)
        check(f"a malformed --click-at {bad!r} is refused", False)
    except ValueError:
        check(f"a malformed --click-at {bad!r} is refused", True)

try:
    timed_plan(None, None, ["20"])
    check("a malformed --key-at is refused", False)
except ValueError:
    check("a malformed --key-at is refused", True)

check("no timed arguments is an empty plan", timed_plan(None, None, None) == [])

print("the timed schedule holds its order")

# ── qmp: a refusal must not read as success ─────────────────────────────────
#
# Every caller ignored the reply, so a refused command - a mistyped qcode, a
# coordinate off the screen - was reported by the driver as a key pressed and a
# click driven. The run then measured a machine nobody had touched.


class _Wire:
    """A QMP peer that answers each command with a canned reply."""

    def __init__(self, replies):
        self._replies = list(replies)
        self.sent = []

    def write(self, data):
        self.sent.append(json.loads(data.decode()))

    def readline(self):
        return (json.dumps(self._replies.pop(0)) + "\n").encode()


refused = _Wire([{"error": {"class": "GenericError", "desc": "invalid parameter 'supr'"}}])
try:
    qmp(refused, "input-send-event", events=[])
    check("a refused command raises rather than returning", False)
except QmpError as e:
    check("a refused command raises rather than returning", True)
    check("and the message names the command", "input-send-event" in str(e))
    check("and quotes what QEMU said", "invalid parameter 'supr'" in str(e))

accepted = _Wire([{"event": "SHUTDOWN", "data": {}}, {"return": {}}])
check(
    "an accepted command returns past the async events",
    qmp(accepted, "screendump", filename="/tmp/x.png") == {"return": {}},
)

# ── the parser must build at all ────────────────────────────────────────────
#
# Nothing checked this, and on 5 September it broke: a help string gained the
# example `50%,300`, argparse read the `%` as a format specifier, and EVERY
# invocation died in `_check_help` before parsing a single argument. The driver
# was unusable and the only sign was a traceback from inside argparse.
#
# Building the parser is the whole check. It touches every `add_argument` in the
# file, so any future help string with a stray `%` fails here instead of on a boot.
import subprocess as _sp  # noqa: E402
from pathlib import Path as _P  # noqa: E402

_r = _sp.run(
    [sys.executable, str(_P(__file__).resolve().parent / "verify.py"), "--help"],
    capture_output=True, text=True, timeout=60,
)
check("the argument parser builds, so --help runs at all", _r.returncode == 0)
check("and it lists the gestures", "--drag" in _r.stdout and "--double-click" in _r.stdout)

print()
if failures:
    print(f"{len(failures)} failure(s)")
    sys.exit(1)
print("a refused QMP command is loud")
