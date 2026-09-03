#!/usr/bin/env python3
"""Which daemon a boot showed binding which socket, and the line that got it wrong.

The socket table is a fact about which binary serves which socket, and the boot
check is the half that tests it against reality: the run itself says who bound
what. A wrong answer here is not a small one - it accuses a daemon of binding
another's socket, which is the exact fault the table exists to catch, and the
message it prints says a wrong value "sends the next reader to the wrong daemon".

It made that accusation falsely on 3 September. `arlen-notifyd` was reported as
serving `undo-signer.sock`, on a boot where the signer had announced its own
socket perfectly normally. The cause is in the fixture below, kept verbatim from
that serial log: a serial console is ONE shared channel, and under load the
journal interleaves on it, so a single physical line carried three messages -
two truncated notifyd fragments and then the signer's announcement. The check
took the first identifier on the line and gave it a path belonging to the third
message.

So these are the cases: the real interleaved line, an ordinary line, and two
announcements sharing one line - which the old code could not get right even in
principle, because it had one speaker for all of a line's sockets.

Run: python3 dev/vm/test_socket_attribution.py
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from verify import observed_servers  # noqa: E402

FAILURES = []


def check(name, got, want):
    ok = got == want
    print(f"  {'ok  ' if ok else 'FAIL'} {name}")
    if not ok:
        print(f"       got  {got}\n       want {want}")
        FAILURES.append(name)


# Verbatim from the boot that produced the false accusation, with the ANSI colour
# noise dropped: three journal messages on one physical serial line.
INTERLEAVED = (
    "[    8.593436] arlen-notifyd[621]: INFO arlen_notifyd: s"
    "[    8.615845] arlen-notifyd[621]: INF"
    "[    8.620465] arlen-ai-undo-signer[610]: INFO arlen_ai_undo_signer::server: "
    "undo-signer listening socket=/run/user/1000/arlen/undo-signer.sock"
)

ORDINARY = (
    "[   10.457908] arlen-shell[710]: INFO arlen_desktop_shell_lib::launch_service] "
    "launch service listening on /run/user/1000/arlen/launch.sock"
)

TWO_ON_ONE = (
    "[1] arlen-notifyd[6]: INFO listening on /run/user/1000/arlen/notification.sock "
    "[2] arlen-ai-undo-signer[9]: INFO listening on /run/user/1000/arlen/undo-signer.sock"
)

# A line with a socket and a speaker but no word that means BINDING: a client
# saying where it is dialling is not a server saying what it serves.
DIALLING = (
    "[   12.0] arlen-notifyd[621]: INFO connecting to /run/user/1000/arlen/undo-signer.sock"
)


def main():
    print("socket attribution")
    check(
        "an interleaved line credits the daemon that announced the socket",
        sorted(observed_servers(INTERLEAVED)),
        [("undo-signer.sock", "arlen-ai-undo-signer")],
    )
    check(
        "an ordinary line still resolves, through the journal alias",
        sorted(observed_servers(ORDINARY)),
        [("launch.sock", "arlen-desktop-shell")],
    )
    check(
        "two announcements on one line go to two daemons",
        sorted(observed_servers(TWO_ON_ONE)),
        [
            ("notification.sock", "arlen-notifyd"),
            ("undo-signer.sock", "arlen-ai-undo-signer"),
        ],
    )
    check(
        "a daemon dialling a socket is not a daemon serving it",
        sorted(observed_servers(DIALLING)),
        [],
    )

    if FAILURES:
        print(f"\n{len(FAILURES)} check(s) failed: {', '.join(FAILURES)}")
        return 1
    print("\nevery line was read as the boot meant it")
    return 0


if __name__ == "__main__":
    sys.exit(main())
