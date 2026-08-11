#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A repo path named in a comment has to be a file that exists.

Prose rots differently from code. A comment saying "see
`apps/settings/src-tauri/src/toml_writer.rs`" was true when written; the file
moved to `apps/settings/core/`, nothing failed, and the note now sends the next
reader to a path that is not there. **Nothing else in the tree can catch this**:
it compiles, the tests pass, and the sentence still reads as if it were checked.

Written on 11 Aug after I did it myself in the same session I fixed it in - a
pointer at `permissions/0/dev.arlen.desktop-shell.toml`, added above the shell's
subscription list, made false two ticks later by my own move of those files. One
mistake in a morning is a mistake; the same mistake twice in a morning is a
missing check.

**Only paths that name a FILE, and that was the whole design problem.** The first
version matched any `dir/word` shape and produced 65 findings, essentially all
prose: `dev/null`, `dev/A` and `dev/B` from a Bluetooth pairing comment,
`apps/AI`, `forage/flatpak/apt` listing three package systems. A check with 65
false positives is one nobody reads, which is the failure mode this whole
directory tries to avoid. Requiring a known file extension took it to 28 checked
and 7 candidates, of which 4 were real.

The remaining three are in KNOWN because they are correct: they name a file in
somebody ELSE's tree - a recipe in a user's project, a state file under a user's
app dir - and this repo is the wrong place to look for those.
"""

import re
import sys
from pathlib import Path

ROOT = (
    Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
)

# A repo-relative path with a file extension we recognise, inside a comment. The
# extension is what separates a path from a sentence: `dev/null` and `apps/AI`
# are prose, `dev/i18n-baseline.tsv` is a claim about this tree.
#
# `tsv` precedes `ts` because Python's alternation takes the FIRST match, not
# the longest: with `ts` first this reads `dev/i18n-baseline.tsv` as a broken
# reference to `dev/i18n-baseline.ts`. It reported exactly that against a path
# I had just corrected, which is how the ordering was found.
#
# The left edge is a negative lookbehind rather than `\b`, because `-` counts as
# a word boundary: `\bsdk/` happily matches the tail of `@arlen/module-sdk/…`,
# turning an npm specifier into a claim that `sdk/postmsg.ts` is missing. Found
# by running the same pattern over TypeScript, where package names are common;
# the `.rs` files it shipped against simply had no such string in them.
PATH = re.compile(
    r"(?<![A-Za-z0-9_/-])((?:apps|daemons|sdk|dev|contracts|ai|forage|store-backend)"
    r"/[A-Za-z0-9_./-]+\.(?:rs|toml|md|py|sh|mjs|tsv|ts|svelte|service|json|proto|yaml)\b)"
)

# path -> why it names something outside this tree, or nothing at all.
KNOWN = {
    # These two exist only as EXAMPLES OF WHAT IS NOT A PATH, in this check's own
    # explanation of its pattern and in its tests: `dev/i18n-baseline.ts` is the
    # misreading of `dev/i18n-baseline.tsv` that ordered alternation produced, and
    # `sdk/postmsg.ts` is the tail of the npm specifier `@arlen/module-sdk/…` that
    # a `\b` left-edge matched. Prose about paths is the price of reading files
    # that explain path rules, and naming the price is cheaper than not reading
    # them.
    "dev/i18n-baseline.ts": "an example of a MISREAD path, quoted in this file's own docstring",
    "sdk/postmsg.ts": "an example of a NON-path, quoted in this file and its tests",
    "forage/recipe.toml": (
        "the recipe in a USER's project that `forage build` reads, not a file here"
    ),
    "apps/skipped-updates.toml": (
        "state the store writes under the user's app directory at runtime"
    ),
    "apps/installed.lock": (
        "the store's runtime lock under the user's app directory, not a repo file"
    ),
}



def scanned_files(root: Path):
    """Every file whose comments can name a repo path.

    It read only `*.rs` at first, which was 27 paths. The rest of the tree writes
    the same kind of note in the same kind of comment - a gate script explaining
    which file it points at, a shell step naming the unit it stages, a Svelte
    store naming its backend - and there are **four times as many** of those. A
    stale path misleads the next reader identically whichever language it sits in,
    so the language it sits in is not a reason to skip it.
    """
    for pattern in ("*.rs", "*.py", "*.mjs", "*.sh", "*.ts", "*.svelte", "*.chroot"):
        yield from root.rglob(pattern)


def comment_paths(text: str):
    """`(line number, path)` for every repo path named in a comment."""
    for i, line in enumerate(text.splitlines(), 1):
        stripped = line.strip()
        if not (stripped.startswith("//") or stripped.startswith("#")):
            continue
        for m in PATH.finditer(stripped):
            yield i, m.group(1).rstrip(".,;:`)")


def main() -> int:
    problems: list[str] = []
    checked = 0
    for path in sorted(scanned_files(ROOT)):
        sp = str(path)
        if "/target/" in sp or "mkosi.builddir" in sp or "/node_modules/" in sp:
            continue
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        for line_no, ref in comment_paths(text):
            if ref in KNOWN:
                continue
            checked += 1
            if not (ROOT / ref).exists():
                rel = path.relative_to(ROOT)
                problems.append(f"{rel}:{line_no}: names `{ref}`, which is not in the tree")

    if problems:
        print("comments pointing at files that are not there:\n")
        for p in problems:
            print(f"  {p}")
        print("\n  Fix the path, or add it to KNOWN with the reason it names something")
        print("  outside this repo. A note nobody can follow is worse than no note.")
        return 1

    print(f"OK: {checked} repo file path(s) named in comments, all present")
    return 0


if __name__ == "__main__":
    sys.exit(main())
