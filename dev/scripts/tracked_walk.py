# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""The gates read what git knows about, not whatever happens to be on disk.

Importing this module makes `Path.rglob`, `Path.glob` and `os.walk` skip everything git
ignores. Every gate that walks the tree imports it on one visible line.

WHY. On 8 October the image build's cache was kept inside the repository, and
every gate that walked the tree with `rglob` read cargo's registry sources in it as
our code: tokio's ignored tests failed `check-tests-run`, a NUL byte in a ron
fixture failed `check-source-is-text`, and every commit in the tree went red over
files nobody here wrote. The same thing happens for any ignored directory a tool
puts in the tree - a `target/`, a `node_modules`, a scratch checkout - so the rule
the planner set is that a gate checks what git tracks.

WHAT COUNTS. `git ls-files --cached --others --exclude-standard`: everything
tracked, plus a new file that is not ignored but not yet added. The second half
matters at commit time: a file being committed is in the index, but a file someone
created and has not staged yet is still the tree a gate should be honest about.
What drops out is exactly what `.gitignore` names.

WHY A PATCH AND NOT A HELPER. 116 gates walk the tree, through a dozen receiver
shapes (`base.rglob`, `sorted(ROOT.rglob(...))`, `(REPO / x).rglob`, `os.walk`).
Rewriting every call site is where a gate gets missed; one import line per gate,
checked by `check-gates-read-tracked.py`, is not.

OUTSIDE A GIT TREE nothing changes. The gates' controls copy a fixture into a temp
directory and run the gate against it; there is no repository there to ask, so
the plain walk is the right answer.
"""

from __future__ import annotations

import os
import pathlib
import subprocess
from functools import lru_cache

_ORIGINAL_RGLOB = pathlib.Path.rglob
_ORIGINAL_GLOB = pathlib.Path.glob
_ORIGINAL_WALK = os.walk


@lru_cache(maxsize=None)
def _known(top: str) -> tuple[str, frozenset[str], frozenset[str]] | None:
    """`(root, files, dirs)` for the repository holding `top`, or None outside one.

    `files` are repo-relative POSIX paths git knows; `dirs` every directory that
    holds one of them, so a walk can tell a directory worth entering from one git
    ignores.
    """
    try:
        root = subprocess.run(
            ["git", "-C", top, "rev-parse", "--show-toplevel"],
            capture_output=True, text=True, check=True,
        ).stdout.strip()
        out = subprocess.run(
            ["git", "-C", root, "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
            capture_output=True, text=True, check=True,
        ).stdout
    except (OSError, subprocess.CalledProcessError):
        return None
    files = frozenset(p for p in out.split("\0") if p)
    dirs: set[str] = {""}
    for f in files:
        parent = f
        while "/" in parent:
            parent = parent.rsplit("/", 1)[0]
            if parent in dirs:
                break
            dirs.add(parent)
    return root, files, frozenset(dirs)


def _rel(root: str, path: str) -> str | None:
    rel = os.path.relpath(os.path.realpath(path), os.path.realpath(root))
    if rel == ".":
        return ""
    if rel.startswith(".."):
        return None
    return rel.replace(os.sep, "/")


def _filtered(original):
    def walk(self: pathlib.Path, pattern: str, *args, **kwargs):
        known = _known(str(self) if self.is_dir() else str(self.parent))
        if known is None:
            yield from original(self, pattern, *args, **kwargs)
            return
        root, files, dirs = known
        for p in original(self, pattern, *args, **kwargs):
            rel = _rel(root, str(p))
            if rel is None or rel in files or rel in dirs:
                yield p

    return walk


def _walk(top, *args, **kwargs):
    known = _known(os.fspath(top))
    if known is None:
        yield from _ORIGINAL_WALK(top, *args, **kwargs)
        return
    root, files, dirs = known
    for dirpath, dirnames, filenames in _ORIGINAL_WALK(top, *args, **kwargs):
        here = _rel(root, dirpath)
        if here is None:
            yield dirpath, dirnames, filenames
            continue
        prefix = f"{here}/" if here else ""
        # Pruned in place, so os.walk does not descend into what git ignores.
        dirnames[:] = [d for d in dirnames if f"{prefix}{d}" in dirs]
        filenames = [f for f in filenames if f"{prefix}{f}" in files]
        yield dirpath, dirnames, filenames


pathlib.Path.rglob = _filtered(_ORIGINAL_RGLOB)  # type: ignore[method-assign]
pathlib.Path.glob = _filtered(_ORIGINAL_GLOB)  # type: ignore[method-assign]
os.walk = _walk  # type: ignore[assignment]

