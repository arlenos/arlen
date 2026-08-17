#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Ask the guest's graph store directly what this boot put in it.

The sibling of `ingest_verdict`, one hop further along. That one answers "did the
`file.opened` reach the event store" in SQL, on the host, with nothing in the
guest getting a vote. This one answers the next question - "did it become a File
node, and did the agent's write really link it to a project" - the same way.

Its docstring used to say the graph half could not be done: a byte search of the
graph store finds schema strings and not values, so a miss would mean either "not
stored verbatim" or "never written", and a gate whose failure has two meanings is
worse than no gate. That is true of a byte search. It is not true in general - the
engine that wrote the store is in this repository, so the host can open the copy
and ask properly. `arlen-graph-verdict` does the asking; this locates the store,
locates the binary and turns the result into a verdict.

Why it matters: `DOGFOOD WRITE ok` is the AGENT's account of its own write. The
dogfood asks it over D-Bus for a completed action and prints the answer, and
`ai_verdict` greps the journal for the line. Nothing reads the edge. A component
reporting wrongly about itself is exactly what that chain cannot catch.
"""

import glob
import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ingest_verdict import DOGFOOD_PATH  # noqa: E402  the one definition of the path

#: Where the graph store lands in the guest, most likely first. `pick_data_path`
#: prefers the per-user directory when HOME is set, and the desktop runs the
#: daemon as a user service, so that is what a boot produces; the system path is
#: what a root-run daemon would write.
GRAPH_STORE_PATHS = (
    "/home/*/.local/share/arlen/graph",
    "/var/lib/arlen/knowledge/graph",
)

#: Where the reader is built. Not installed into the image: it runs on the host,
#: against a copy, which is the whole point.
BINARY = "daemons/knowledge/target/debug/arlen-graph-verdict"
BINARY_ALT = "target/debug/arlen-graph-verdict"


def find_binary(repo: str) -> str | None:
    """The built verdict reader, or None if nobody built it."""
    for rel in (BINARY_ALT, BINARY):
        p = os.path.join(repo, rel)
        if os.path.exists(p):
            return p
    return None


def run(store: str, repo: str, path: str = DOGFOOD_PATH) -> tuple[int, str]:
    """(exit code, output) of the reader. 0 confirmed, 1 refused, 2 not measured.

    The three-way code is kept whole here rather than reduced to a boolean,
    because "the store says no" and "nothing was read" are different facts and
    collapsing them is how a check that did not run comes to look like a pass.

    A missing reader is one of the not-measured cases, not a pass, so it reports
    the command that fixes it.
    """
    binary = find_binary(repo)
    if binary is None:
        return 2, (
            "GRAPH UNREADABLE: the reader is not built, so nothing was measured. "
            "Build it: cargo build --manifest-path daemons/knowledge/Cargo.toml "
            "--bin arlen-graph-verdict"
        )

    r = subprocess.run([binary, store, "--file", path], capture_output=True, text=True)
    return r.returncode, (r.stdout + r.stderr).strip()


def graph_verdict(store: str, repo: str, path: str = DOGFOOD_PATH) -> tuple[bool, str]:
    """(ok, message), the shape `ingest_verdict` has and verify.py grades.

    Both refused and not-measured are failures to the boot run, which is right:
    it must not pass on a graph nobody managed to read. The distinction survives
    in the message and in `run`'s code for anyone calling this directly.
    """
    code, out = run(store, repo, path)
    return code == 0, out


#: What guestfish says when the glob matched nothing. Any OTHER failure is the
#: tooling not working, which is a different fact from the guest not having a
#: store - see `copy_out`.
NOT_THERE = "is not a file or directory"


def copy_out(overlay: str, dest_root: str) -> tuple[str | None, str | None]:
    """(store path, unreadable reason) for the guest's graph store.

    Its own guestfish call per candidate path, like the event store's: a glob that
    matches nothing makes guestfish exit non-zero, so folding several into one
    script would let a moved store discard the others.

    The two-value return exists because a glob that matches nothing and a
    libguestfs appliance that will not start BOTH exit non-zero. The first cut
    returned None for either, and the caller says "the guest wrote no graph
    store" - a claim about the guest drawn from a fact about the host. That is
    not hypothetical: the appliance failed to launch on this machine tonight,
    under load, in the pre-commit hook.

    So absence is only concluded when every candidate failed with the not-found
    message; anything else is reported as unreadable and the run does not get to
    call it an answer.
    """
    problems: list[str] = []
    for i, guest_path in enumerate(GRAPH_STORE_PATHS):
        dest = os.path.join(dest_root, str(i))
        os.makedirs(dest, exist_ok=True)
        script = f"run\nmount-ro /dev/sda2 /\nglob copy-out {guest_path} {dest}/\n"
        r = subprocess.run(
            ["guestfish", "--ro", "-a", overlay],
            input=script,
            capture_output=True,
            text=True,
        )
        if r.returncode != 0:
            err = (r.stderr or "").strip()
            if NOT_THERE not in err:
                problems.append(f"{guest_path}: {err.splitlines()[-1] if err else 'guestfish failed'}")
            continue
        found = glob.glob(os.path.join(dest, "graph"))
        if found:
            return found[0], None
    if problems:
        return None, "the graph store could not be read out of the image: " + "; ".join(problems)
    return None, None


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("usage: graph_verdict.py <store-path> [repo-root]", file=sys.stderr)
        raise SystemExit(2)
    repo_root = sys.argv[2] if len(sys.argv) > 2 else os.path.abspath(
        os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
    )
    code, message = run(sys.argv[1], repo_root)
    print(message)
    raise SystemExit(code)
