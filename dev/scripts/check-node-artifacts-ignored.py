# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that every Node package in the tree has an ignored `node_modules`.

A build phase that builds a frontend does it IN THE CHECKOUT: `BuildSources`
mounts the repository into the container, so `npm ci` writes into the real
directory. If nothing ignores that, one image build leaves thousands of untracked
files behind, and the next `git add -A` commits them - which has already happened
once in this tree, when 8589 build artefacts across five `target/` directories
turned out to be tracked.

Every package under `apps/` and `sdk/` carries its own `.gitignore` and was fine.
The two that were not are the two nobody thinks of as frontends: `ai/pi-plugins`,
which sits under a Rust `.gitignore` and is the only Node package there, and
`daemons/xdg-portal/picker-ui`, which sits under a daemon. Both are built in
place by an image phase.

So the rule is about the PACKAGE, not the directory it happens to live in: a
`package.json` outside an already-ignored tree means an `npm ci` can happen there,
and `node_modules` beside it must be ignored.

What this does NOT check: the build OUTPUT directory, because its name is the
package's own choice (`dist`, `build`, `.svelte-kit`) and reading it out of a
tsconfig or a vite config to check it would be a second guess about a moving
target. `node_modules` is the one every Node package has by the same name.

Shown to fail before being trusted: the control plants a package with nothing
ignoring it.
"""

import subprocess
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]


def ignored(path: Path) -> bool:
    """Whether git would ignore this path, asked of git rather than guessed."""
    r = subprocess.run(
        ["git", "-C", str(ROOT), "check-ignore", "-q", str(path)],
        capture_output=True,
    )
    return r.returncode == 0


def main() -> int:
    if not (ROOT / ".git").exists():
        print(f"{ROOT} is not a git checkout; this check asks git what it ignores")
        return 1

    packages = [
        p
        for p in ROOT.rglob("package.json")
        if "node_modules" not in p.parts and not ignored(p.parent)
    ]
    if not packages:
        print(f"no Node packages under {ROOT}; the layout moved and this check did not")
        return 1

    problems = []
    for pkg in sorted(packages):
        mods = pkg.parent / "node_modules"
        if ignored(mods):
            continue
        problems.append(
            f"{pkg.parent.relative_to(ROOT)} is a Node package and nothing ignores its "
            f"node_modules. An image phase that builds it runs `npm ci` inside the "
            f"checkout, so one build leaves thousands of untracked files here and the "
            f"next `git add -A` commits them."
        )

    print(f"{len(packages)} Node package(s): each has an ignored node_modules beside it.")
    if problems:
        print("\npackages whose install would land in git:\n")
        for p in problems:
            print(f"  - {p}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
