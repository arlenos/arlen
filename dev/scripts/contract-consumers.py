# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Which crates would break if this one changed.

Adding a field to a contract crate cannot be validated by building that crate:
the breakage is by definition in someone else. That is not a hypothetical - four
crates went red in one night from two field additions, each of which built
cleanly where it was made, and the tree has no root workspace to catch it.

So this walks the path dependencies backwards. Every `Cargo.toml` under the repo
declares its dependencies as relative paths; resolving those gives a graph, and
reversing it gives the answer to "who do I have to build before I commit this".
Transitive, because a consumer's consumer breaks just as hard.

Usage:
    contract-consumers.py <crate-path>...   the consumers of these crates
    contract-consumers.py --changed         the consumers of whatever the working
                                            tree has touched, which is the form
                                            worth running as a habit

Prints one crate path per line, so a caller can loop over it. Prints nothing and
exits 0 when there are none, which is the common case and not an error.
"""

import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]

# `name = { path = "../../contracts/audit-proto" }` or a `[dependencies.x]`
# table's `path = "..."`. Both forms appear in the tree.
DEP_PATH = re.compile(r'path\s*=\s*"([^"]+)"')


def manifests() -> list[pathlib.Path]:
    """Every crate manifest in the repo, skipping build output and vendored trees."""
    out = []
    for m in ROOT.rglob("Cargo.toml"):
        parts = set(m.parts)
        if "target" in parts or "node_modules" in parts or ".git" in parts:
            continue
        out.append(m)
    return out


def crate_dir(manifest: pathlib.Path) -> pathlib.Path:
    return manifest.parent


def dependants() -> dict[pathlib.Path, set[pathlib.Path]]:
    """crate directory -> the crate directories that depend on it directly."""
    rev: dict[pathlib.Path, set[pathlib.Path]] = {}
    for m in manifests():
        here = crate_dir(m)
        for raw in DEP_PATH.findall(m.read_text()):
            # `path = "src/lib.rs"` in a [lib] table is not a dependency.
            if raw.endswith(".rs"):
                continue
            target = (here / raw).resolve()
            if (target / "Cargo.toml").is_file():
                rev.setdefault(target, set()).add(here)
    return rev


def consumers(seeds: list[pathlib.Path]) -> list[pathlib.Path]:
    """Every crate that depends on any of `seeds`, transitively, excluding them."""
    rev = dependants()
    seen: set[pathlib.Path] = set()
    queue = list(seeds)
    while queue:
        crate = queue.pop()
        for dep in rev.get(crate, ()):
            if dep not in seen:
                seen.add(dep)
                queue.append(dep)
    return sorted(seen - set(seeds))


def changed_crates() -> list[pathlib.Path]:
    """The crates the working tree has touched, against the merge base."""
    # `status --porcelain`, not `diff`: a new file in a crate is exactly as
    # capable of needing its consumers rebuilt, and `diff` does not see untracked
    # ones. The two-character status prefix is stripped, and a rename's `old ->
    # new` keeps the new name.
    lines = subprocess.run(
        ["git", "status", "--porcelain", "--untracked-files=all"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    ).stdout.splitlines()
    files = [ln[3:].split(" -> ")[-1].strip('"') for ln in lines if len(ln) > 3]
    out: set[pathlib.Path] = set()
    for f in files:
        d = (ROOT / f).parent
        # Walk up to the nearest crate root, so a change three directories deep
        # inside `src/` still names its crate.
        while d != ROOT and d.is_relative_to(ROOT):
            if (d / "Cargo.toml").is_file():
                out.add(d)
                break
            d = d.parent
    return sorted(out)


def main(argv: list[str]) -> int:
    if not argv:
        print(
            "usage: contract-consumers.py <crate-path>... | --changed",
            file=sys.stderr,
        )
        return 2
    if argv == ["--changed"]:
        seeds = changed_crates()
    else:
        seeds = []
        for a in argv:
            p = (ROOT / a).resolve()
            if not (p / "Cargo.toml").is_file():
                print(f"contract-consumers: no Cargo.toml at {a}", file=sys.stderr)
                return 2
            seeds.append(p)
    for c in consumers(seeds):
        print(c.relative_to(ROOT))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
