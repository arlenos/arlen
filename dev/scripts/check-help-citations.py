# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that command-line help does not cite a document the reader cannot open.

`forage --help` printed four of these until 17 August:

    build      Build a recipe into a `.lunpkg` (build pipeline, forage-recipes.md R1)
    challenge  Challenge a build's reproducibility (forage-recipes.md section 8a)

The architecture docs are a SEPARATE PRIVATE repository. Whoever types
`forage --help` cannot follow those references, and two of the four leaned on the
citation instead of explaining - "Challenge a build's reproducibility" says almost
nothing to someone who does not already know what a challenge is.

The reason this hides: a `///` on a clap type is user-facing text that does not
look like user-facing text. It sits among ordinary doc comments in a source file
and gets reviewed as documentation. The fix is always the same - describe the
command in the `///`, and put the design reference on a `//` line above, which
clap does not print.

Two shapes are checked, because the tree has both:

  * `///` inside a file that derives clap (`forage/src/cli.rs`)
  * a usage/help string built by hand (`trash-rm`, whose parser is hand-rolled)

Not checked: `///` on ordinary functions. `arlen-run`'s `spawn.rs` cites plans to
a maintainer reading the source, which is exactly where a citation belongs.
"""

import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

#: A reference to a repo document. `.md` is the whole giveaway: no user-facing
#: sentence needs a filename in it.
CITATION = re.compile(r"[\w./-]+\.md\b")

#: The derives that turn a `///` into `--help` output.
CLAP_DERIVE = re.compile(r"derive\([^)]*\b(Parser|Subcommand|Args|ValueEnum)\b")

#: A hand-written help string. `trash-rm` builds one as a const.
USAGE_TEXT = re.compile(r"Usage:\s", re.I)

#: A whole `const NAME: &str = "...";` block, so its body can be judged as a
#: unit rather than guessed at line by line.
CONST_BLOCK = re.compile(r'const\s+\w+\s*:\s*&\s*str\s*=\s*"(?:[^"\\]|\\.)*"\s*;', re.S)

SKIP = ("/target/", "/node_modules/", "mkosi.builddir", "/.git/")


def main() -> int:
    findings: list[str] = []
    scanned = 0

    for path in sorted(ROOT.rglob("*.rs")):
        s = str(path)
        if any(x in s for x in SKIP):
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        is_clap = bool(CLAP_DERIVE.search(text))
        has_usage = bool(USAGE_TEXT.search(text))
        if not (is_clap or has_usage):
            continue
        scanned += 1
        rel = path.relative_to(ROOT)

        # Hand-written help is a string CONSTANT, so pull the whole block out and
        # look only at the ones whose body IS help. Two earlier cuts tracked this
        # line by line and both leaked: the first latched on at any "Usage:" and
        # never let go (reporting a theme token named `spacing.md`, md as in
        # medium), the second entered on a ONE-LINE const whose closing quote it
        # had already skipped, and read the rest of the file as help.
        checked_regions: list[tuple[int, str]] = []
        for m in CONST_BLOCK.finditer(text):
            if not USAGE_TEXT.search(m.group(0)):
                continue
            first = text.count("\n", 0, m.start()) + 1
            for offset, line in enumerate(m.group(0).splitlines()):
                checked_regions.append((first + offset, line))

        if is_clap:
            for n, line in enumerate(text.splitlines(), 1):
                if line.strip().startswith("///"):
                    checked_regions.append((n, line))

        for n, line in sorted(checked_regions):
            hit = CITATION.search(line)
            if hit:
                findings.append(
                    f"{rel}:{n}: help text cites {hit.group(0)}: {line.strip()[:72]}"
                )

    if not scanned:
        print("NOTHING WAS READ: no file builds command-line help", file=sys.stderr)
        return 2

    print(
        f"{scanned} file(s) that build command-line help checked for references to "
        f"repo documents. The docs repo is private, so a citation in `--help` is one "
        f"the reader cannot follow."
    )
    if findings:
        print("\nhelp text pointing at something the reader cannot open:\n", file=sys.stderr)
        for f in findings:
            print(f"  - {f}", file=sys.stderr)
        print(
            "\nSay what the command does in the `///`, and move the design reference "
            "to a `//` line above it - clap prints the first and ignores the second.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
