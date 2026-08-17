# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a README's directory tree names directories the reader will get.

The root README drew this until 17 August:

    arlen/
      dev/         build, test and dev tooling
      docs/        architecture specs (shared across the tree)

`docs/` is gitignored. The architecture specs are a separate private repository
cloned into that path for local work, so every public clone arrives without it.
The most-read file in the repo promised a directory that has never once been
delivered, and it said so in the layout block, which is the part a newcomer
reads first.

It survived the restructure and months after it because a tree block is prose
to a reviewer and a claim to a reader. Nothing compared it against the tree.

Four blocks exist today and they are drawn three different ways: the root README
indents with spaces, `apps/settings` nests with two-space indent under TWO roots
in one block, and `dev` and `sdk/ui-kit` use box-drawing. All three are parsed
here, because a check that only understood the shape in front of it would have
passed the other three without reading them.

Entries resolve against the README's own directory. A top-level entry that names
the repo itself resolves to the repo root instead, which is how `arlen/` works.
"""

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

#: A fenced block whose first content line is a bare `name/` is a directory tree.
FENCE = re.compile(r"^```[^\n]*\n(.*?)^```", re.S | re.M)

#: The box-drawing a tree is often drawn with. Each one is worth one level.
BRANCH = re.compile(r"^(?:[│├└][─ ]*\s?)")

#: An entry line: a path, then whatever description follows.
ENTRY = re.compile(r"^([\w.@/-]+/?)(?:\s|$)")


def levels(line: str) -> tuple[int, str]:
    """The column an entry's name starts at, and the entry itself.

    Ranking by column rather than by counting levels is what makes one parser
    read all three shapes. A two-space tree, a four-space tree and a box-drawing
    tree disagree about what one level looks like but agree that a child starts
    further right than its parent. Counting levels needs a rule per shape, and
    the first cut of this got two of them wrong: it inverted the comparison for
    space-indented trees, and it read `    └── src/` as depth one because the
    four spaces standing in for the level above were stripped and not counted.
    """
    rest = line
    while True:
        stripped = rest.lstrip(" ")
        m = BRANCH.match(stripped)
        if not m:
            break
        rest = stripped[m.end() :]
    rest = rest.lstrip(" ")
    return len(line) - len(rest), rest.strip()


def parse(block: str) -> list[str]:
    """Every path a tree block claims, relative to the block's own root."""
    stack: list[tuple[int, str]] = []
    out: list[str] = []
    for raw in block.splitlines():
        if not raw.strip():
            continue
        rank, rest = levels(raw)
        m = ENTRY.match(rest)
        if not m:
            continue
        name = m.group(1)
        while stack and stack[-1][0] >= rank:
            stack.pop()
        parent = stack[-1][1] if stack else ""
        full = parent + name
        if name.endswith("/"):
            stack.append((rank, full))
        out.append(full)
    return out


def tracked_paths() -> set[str]:
    """Every path a clone receives, files and the directories holding them.

    Membership here, not `Path.exists()`. The defect that prompted this check is
    a directory that IS on the machine that wrote the README - `docs/` is the
    private specs repo cloned in - and is not in the repository. Asking the
    filesystem answers with the author's machine, so the first cut of this check
    passed with the defect deliberately restored. Only git knows what a clone
    actually gets.
    """
    out = subprocess.run(
        ["git", "-C", str(ROOT), "ls-files"], capture_output=True, text=True
    ).stdout.split()
    paths = set(out)
    for f in out:
        parent = Path(f).parent
        while str(parent) != ".":
            paths.add(str(parent))
            parent = parent.parent
    return paths


def main() -> int:
    files = subprocess.run(
        ["git", "-C", str(ROOT), "ls-files", "*.md"], capture_output=True, text=True
    ).stdout.split()
    tracked = tracked_paths()

    findings: list[str] = []
    blocks = 0
    entries = 0

    for rel in files:
        path = ROOT / rel
        try:
            text = path.read_text(encoding="utf-8")
        except OSError:
            continue
        base = path.parent

        for block in FENCE.findall(text):
            lines = [l for l in block.splitlines() if l.strip()]
            if not lines or not re.fullmatch(r"[\w.-]+/", lines[0].strip()):
                continue
            blocks += 1
            claims = parse(block)
            if not claims:
                continue

            # The first line of a tree is its root, and it is a LABEL as often as
            # it is a path: the repo README heads its block `arlen/`, and
            # `sdk/ui-kit` heads its own with `ui-kit/`, neither of which names a
            # directory inside the tree they are drawing. `apps/settings` heads
            # its block with `src/`, which does. The rule that separates them
            # without a list: if the head resolves to something the repo tracks
            # it is a path, and otherwise it is a name for where we already are.
            label = claims[0]
            head = str((base / label).relative_to(ROOT))
            if head in tracked:
                prefix, rest = base, claims
            else:
                prefix, rest = base, [c[len(label) :] for c in claims[1:]]

            for claim in rest:
                if not claim:
                    continue
                target = (prefix / claim).relative_to(ROOT)
                entries += 1
                if str(target) not in tracked:
                    findings.append(f"{rel}: draws {claim}, which a clone does not get ({target})")

    if not blocks:
        print("NOTHING WAS READ: no README draws a directory tree", file=sys.stderr)
        return 2

    print(
        f"{entries} entr(ies) across {blocks} README tree block(s) checked against the "
        f"tree. A block is the first thing a newcomer reads, so a directory drawn "
        f"there is a promise about what a clone contains."
    )
    if findings:
        print("\nREADME trees drawing something a clone does not get:\n", file=sys.stderr)
        for f in findings:
            print(f"  - {f}", file=sys.stderr)
        print(
            "\nRemove it from the block, or say in the prose where it actually comes "
            "from - `docs/` is a private repo cloned in, not part of this one.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
