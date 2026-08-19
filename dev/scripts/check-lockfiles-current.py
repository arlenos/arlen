#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Check that every committed lockfile describes its committed manifest.

WHY. On 19 August four app lockfiles were stale at once - `apps/clock`,
`apps/knowledge`, `apps/meetings`, `apps/store`. The shared shell plugin had
gained `zbus`, the manifests said so, and the locks beside them did not: 199
lines of the zbus tree missing from one of them. Nothing was red, because
nothing looks. CI does not pass `--locked`, so cargo quietly re-resolves,
builds fine, and throws the answer away when the job ends - and the stale file
stays stale in the tree for as long as nobody happens to run a build and commit
the result.

WHAT IT COSTS TO LEAVE. A lockfile is the record of what was actually built. A
stale one means the tree cannot be built reproducibly from what is committed:
anything that pins - an offline build, an image build, a `--locked` release,
somebody bisecting - resolves differently or refuses. It is also how a
dependency arrives without anybody reviewing it, since the diff that would have
shown the new tree never appears.

HOW. `cargo metadata --locked` fails when the lock would have to change, which
is exactly the question. `--offline` keeps it off the network, so a warm cargo
cache answers in about eight seconds for the whole tree.

THE TWO FAILURES ARE NOT THE SAME and this says which it saw. A lock that needs
updating is a red. A crate missing from the local cache means the check could
not run here, which is a skip: reporting a cold cache as drift would be the
gate lying, and one that cries wolf on a fresh checkout is one people learn to
ignore.

Run: dev/scripts/check-lockfiles-current.py [root]
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

#: cargo's own wording when the lock would have to move. Matched rather than
#: guessed at from the exit code, because every other failure exits the same way.
STALE = "because --locked was passed"

#: What a cold cache says. Distinguished so it reads as "not checked" rather
#: than "wrong", which are different facts about the tree.
COLD = ("no matching package named", "failed to load source", "not found in registry")


def lockfiles(root: Path) -> list[Path]:
    """Every committed lockfile, newest-first order irrelevant."""
    out = subprocess.run(
        ["git", "ls-files", "*Cargo.lock"],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    return [root / line for line in out.stdout.split("\n") if line.strip()]


def check(manifest: Path) -> tuple[str, str]:
    """`("ok"|"stale"|"skipped", detail)` for one manifest."""
    r = subprocess.run(
        [
            "cargo",
            "metadata",
            "--manifest-path",
            str(manifest),
            "--format-version",
            "1",
            "--locked",
            "--offline",
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    if r.returncode == 0:
        return ("ok", "")
    err = r.stderr
    if STALE in err:
        return ("stale", "the lockfile would have to change to match the manifest")
    if any(c in err for c in COLD):
        return ("skipped", "not in the local cargo cache")
    return ("skipped", err.strip().split("\n")[0] if err.strip() else "cargo said nothing")


def main(argv: list[str]) -> int:
    root = Path(argv[1]).resolve() if len(argv) > 1 else Path(__file__).resolve().parents[2]
    locks = lockfiles(root)
    if not locks:
        print(f"NOTHING WAS READ: no lockfile is committed under {root}", file=sys.stderr)
        return 2

    stale: list[str] = []
    skipped: list[str] = []
    checked = 0
    for lock in locks:
        manifest = lock.with_name("Cargo.toml")
        if not manifest.is_file():
            continue
        verdict, detail = check(manifest)
        rel = manifest.parent.relative_to(root)
        if verdict == "ok":
            checked += 1
        elif verdict == "stale":
            stale.append(f"{rel} - {detail}")
        else:
            skipped.append(f"{rel} - {detail}")

    if not checked and not stale:
        print(
            "NOTHING WAS READ: every manifest was skipped, so this checked nothing",
            file=sys.stderr,
        )
        for s in skipped:
            print(f"  {s}", file=sys.stderr)
        return 2

    if skipped:
        print(f"not checked here ({len(skipped)}):")
        for s in sorted(skipped):
            print(f"  {s}")
        print()

    if stale:
        print("a lockfile no longer describes the manifest beside it:\n")
        for s in sorted(stale):
            print(f"  - {s}")
        print(
            "\n  Run `cargo metadata --manifest-path <crate>/Cargo.toml --format-version 1`\n"
            "  and commit the lockfile it writes. A stale lock means the tree cannot be\n"
            "  built reproducibly from what is committed, and that a dependency arrived\n"
            "  without a diff anybody read."
        )
        return 1

    print(f"{checked} lockfile(s) describe the manifest beside them.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
