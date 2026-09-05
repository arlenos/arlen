#!/usr/bin/env python3
"""Fixtures for the boot driver's pointer gestures.

A drag and a double click are the two things `qmp_click` cannot express, and both
were being answered by reading source instead of by pressing. What makes them work
is the SHAPE of the event stream, not the endpoints:

  * a drag is only a drag if motion arrives BETWEEN the press and the release. A
    compositor starts an interactive move on the motion that follows the press, so
    a down-then-up at two different points is a click at the second point, however
    far apart they are. That is a silent failure - the run reports a drag driven
    and nothing moved - so it is asserted here.
  * a double click is only a double click if the two presses fall inside the
    interval. The existing click loop sleeps 1.5s between clicks, which is well
    past it, so "click twice" was never available.

Both are checked against a fake QMP peer, which is where they can be checked at
all without a VM.
"""

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from verify import qmp_double_click, qmp_drag  # noqa: E402

failed = 0


def check(name, ok, detail=None):
    global failed
    if ok:
        print(f"  ok   {name}")
    else:
        print(f"  FAIL {name}")
        if detail is not None:
            print(f"       {detail}")
        failed += 1


class _Wire:
    """A QMP peer that accepts everything and remembers what it was sent."""

    def __init__(self):
        self.sent = []

    def write(self, data):
        self.sent.append(json.loads(data.decode()))

    def readline(self):
        return (json.dumps({"return": {}}) + "\n").encode()


def events(wire):
    """Every input event sent, flattened in order."""
    out = []
    for cmd in wire.sent:
        out.extend(cmd.get("arguments", {}).get("events", []))
    return out


def buttons(evs):
    """The button edges, as a list of down/up booleans in order."""
    return [e["data"]["down"] for e in evs if e["type"] == "btn"]


def positions(evs):
    """The absolute positions, as (x, y) pairs in order."""
    xs = [e["data"]["value"] for e in evs if e["type"] == "abs" and e["data"]["axis"] == "x"]
    ys = [e["data"]["value"] for e in evs if e["type"] == "abs" and e["data"]["axis"] == "y"]
    return list(zip(xs, ys))


print("pointer gestures:")

# ── drag ────────────────────────────────────────────────────────────────────
w = _Wire()
qmp_drag(w, 100, 10, 700, 10, 1280, 800, steps=6)
evs = events(w)

check("a drag presses once and releases once", buttons(evs) == [True, False], buttons(evs))

# The load-bearing one: motion between the edges.
down_at = next(i for i, e in enumerate(evs) if e["type"] == "btn" and e["data"]["down"])
up_at = next(i for i, e in enumerate(evs) if e["type"] == "btn" and not e["data"]["down"])
between = [e for e in evs[down_at + 1:up_at] if e["type"] == "abs"]
check("and moves while the button is held", len(between) >= 6, f"{len(between)} moves between the edges")

check(
    "and the last position is the destination, not the start",
    positions(evs)[-1][0] == round(700 * 0x7fff / 1280),
    positions(evs)[-1],
)
check(
    "and the first position is the start",
    positions(evs)[0][0] == round(100 * 0x7fff / 1280),
    positions(evs)[0],
)

# A vertical drag is the resize/move case and must not collapse to the x axis.
w2 = _Wire()
qmp_drag(w2, 400, 100, 400, 500, 1280, 800, steps=4)
ys = [p[1] for p in positions(events(w2))]
check("a vertical drag moves on y", ys[0] != ys[-1], (ys[0], ys[-1]))

# ── double click ────────────────────────────────────────────────────────────
w3 = _Wire()
qmp_double_click(w3, 640, 18, 1280, 800)
evs3 = events(w3)
check("a double click sends two press-release pairs", buttons(evs3) == [True, False, True, False], buttons(evs3))
check(
    "and both land on the same point",
    len(set(positions(evs3))) == 1,
    set(positions(evs3)),
)

if failed:
    print(f"\n{failed} failed")
    sys.exit(1)
print("a drag moves under a held button and a double click is two presses at one point")
