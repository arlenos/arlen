# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Ask the shell's launch socket a few questions and print what it answers.

The launch socket is where a request to start something becomes a process: the
portal, the file manager and the launcher were all moved onto it so that the
confinement decision, the audit record and the handler lookup happen once rather
than three times. It had never served a request, because the shell panicked while
binding it, and a panic in a startup path is invisible to a test suite.

So this exists to drive it. It connects, sends real requests, and prints the
outcomes. It is not a pass/fail gate: what a correct answer looks like depends on
the machine's own handler configuration, and a check that has to guess that would
be asserting the developer's mimeapps rather than the shell's behaviour.

Run it against a shell that is already up:

    python3 dev/scripts/probe-launch-socket.py

What the answers mean:

  * an `app` request refused - EXPECTED from this script. Naming a specific
    application is admitted only for a caller the shell could resolve to a known
    app id, and a python process is not one. It is the `refused:unresolved-caller`
    path, and seeing it means peer attestation is working.
  * an `open` request answered `no_handler` - depends on the environment. It is
    the honest answer when nothing in the visible mimeapps files claims the type.
    Note that `XDG_CONFIG_HOME` decides which mimeapps files are visible, and the
    screenshot harness sets it to a temp dir: under `shoot-compositor.sh` the
    shell cannot see `~/.config/mimeapps.list`, so `no_handler` there says
    nothing about the machine. That cost me a while, hence this paragraph.
  * `started` with an app id - the whole path worked, and something is now
    running.
"""

import json
import os
import socket
import struct
import sys

REQUESTS = [
    ("a named application, from a caller the shell cannot resolve",
     {"kind": "app", "app_id": "does.not.exist"}),
    ("a document, with the type supplied",
     {"kind": "open",
      "target": {"uri": "file:///etc/hostname", "path": "/etc/hostname"},
      "mime": "text/plain"}),
    ("a document, leaving the type to the service",
     {"kind": "open",
      "target": {"uri": "file:///etc/hostname", "path": "/etc/hostname"}}),
]


def ask(path: str, request: dict) -> dict:
    """One request, one framed answer. 4-byte big-endian length, then JSON."""
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
        s.settimeout(5)
        s.connect(path)
        body = json.dumps(request).encode()
        s.sendall(struct.pack(">I", len(body)) + body)
        header = s.recv(4)
        if len(header) < 4:
            raise OSError("the shell closed the connection without answering")
        size = struct.unpack(">I", header)[0]
        buf = b""
        while len(buf) < size:
            chunk = s.recv(size - len(buf))
            if not chunk:
                raise OSError("the answer stopped short")
            buf += chunk
    return json.loads(buf)


def main() -> int:
    runtime = os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}")
    path = os.path.join(runtime, "arlen/launch.sock")
    if not os.path.exists(path):
        print(f"{path} is not there - the shell is not running, or it did not bind it")
        return 2

    print(f"asking {path}\n")
    for label, request in REQUESTS:
        try:
            print(f"  {label}\n    -> {ask(path, request)}")
        except OSError as e:
            print(f"  {label}\n    -> FAILED: {e}")
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
