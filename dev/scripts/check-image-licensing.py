# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that every tracked image is licensed by a block that names it.

`REUSE.toml` opens with a catch-all `path = "**"` declaring the tree as Tim
Kicker's AGPL work. For source that is the right approximation and the per-file
headers refine it. For ARTWORK it is not an approximation, it is a claim: the
stock Tauri logo sat in seventeen `src-tauri/icons/` directories and the
catch-all declared somebody else's mark as our copyright. Nothing failed, because
a bulk rule always matches.

So the rule for images is narrower than for code: a picture must be covered by a
block that is ABOUT it, never only by the catch-all. That is not a licensing
decision waiting on anybody - `themes/adw-gtk3/**` is the precedent in this tree
and it works. Two hundred sidecars for images that are ours would be noise; a
path block per group is the same statement without the noise.

What this does NOT check: whether the licence named is the RIGHT one. A block
saying the Tauri logo is ours would pass. The check makes the claim EXPLICIT, and
an explicit wrong claim is one a reader can see; the catch-all's wrong claim is
invisible by construction.

UNVERIFIED is for a picture whose provenance nobody has established. It is not an
excuse list: an entry is a promise that somebody looked and could not tell, and
the reason is written where the next reader finds it.

Shown to fail before being trusted: the control plants an undeclared image.
"""

import re
import subprocess
import sys
import tomllib
from pathlib import Path

IMAGE_SUFFIXES = {
    ".png", ".jpg", ".jpeg", ".gif", ".svg", ".ico", ".icns",
    ".webp", ".avif", ".bmp", ".tiff", ".tif", ".heic", ".heif", ".jxl",
}

# The catch-all. A block whose only matching pattern is this one does not name
# the image; that is the whole subject of this check.
CATCH_ALL = "**"

# path -> why nobody can declare it yet. Each entry is a picture somebody looked
# at and could not source, kept visible rather than quietly swept under the
# catch-all.
UNVERIFIED: dict[str, str] = {
    "sdk/ui-kit/src-tauri/icons": (
        "a purple crescent that is NOT the Tauri default (the app icons are, and "
        "these differ byte for byte). It arrived in one April commit alongside a "
        "plugin implementation, with no vector source in the tree, no README and "
        "no provenance line, and the flat rounded-cap crescent is the house style "
        "of every stock icon set - so it may well be one, and the old project "
        "name meant moon. Declaring it ours would be the same mistake as the "
        "Tauri logo with a different picture. Nothing ships it (ui-kit's "
        "`src-tauri` is a library harness), so the cost of not knowing is zero "
        "until somebody propagates it"
    ),
}


# Directories a walk never descends into. Only consulted off the git path (a
# fixture tree), where there is no index to ask.
SKIP_DIRS = {".git", "node_modules", "target", ".svelte-kit", "build", "dist"}


def tracked_images(root: Path) -> list[str]:
    """Every tracked file with an image suffix, repo-relative.

    Tracked, not present: a screenshot somebody left in the working tree is not
    something the repo licenses. A fixture tree has no index, so there the walk
    stands in for it - the answer is the same set for a tree with nothing
    ignored in it.
    """
    try:
        out = subprocess.run(
            ["git", "-C", str(root), "ls-files"],
            capture_output=True, text=True, check=True,
        ).stdout.split("\n")
        paths = [p for p in out if p]
    except (subprocess.CalledProcessError, FileNotFoundError):
        paths = []
        for path in root.rglob("*"):
            if not path.is_file():
                continue
            rel = path.relative_to(root)
            if any(part in SKIP_DIRS for part in rel.parts):
                continue
            paths.append(str(rel))
    return sorted(p for p in paths if Path(p).suffix.lower() in IMAGE_SUFFIXES)


def pattern_to_regex(pattern: str) -> re.Pattern[str]:
    """A REUSE path glob as a regex.

    `**` crosses directory separators, `*` does not - the same split REUSE's own
    matcher makes, and the reason `apps/*/src-tauri/icons/**` is a different
    statement from `apps/**`.
    """
    parts: list[str] = []
    i = 0
    while i < len(pattern):
        if pattern.startswith("**", i):
            parts.append(".*")
            i += 2
        elif pattern[i] == "*":
            parts.append("[^/]*")
            i += 1
        else:
            parts.append(re.escape(pattern[i]))
            i += 1
    return re.compile("^" + "".join(parts) + "$")


def blocks(reuse: Path) -> list[tuple[list[str], list[re.Pattern[str]]]]:
    """Every annotation block's patterns, in file order."""
    doc = tomllib.loads(reuse.read_text(encoding="utf-8"))
    out = []
    for block in doc.get("annotations", []):
        raw = block.get("path", [])
        patterns = [raw] if isinstance(raw, str) else list(raw)
        out.append((patterns, [pattern_to_regex(p) for p in patterns]))
    return out


def names_it(path: str, parsed: list[tuple[list[str], list[re.Pattern[str]]]]) -> bool:
    """Does any block match `path` with a pattern that is not the catch-all?"""
    for patterns, regexes in parsed:
        for pattern, regex in zip(patterns, regexes):
            if pattern != CATCH_ALL and regex.match(path):
                return True
    return False


def carries_own_header(root: Path, path: str) -> bool:
    """An SVG can state its own licence, and `precedence = "closest"` means it
    wins over every block. A binary cannot, so this only ever answers for text."""
    if Path(path).suffix.lower() != ".svg":
        return False
    try:
        return "SPDX-License-Identifier" in (root / path).read_text(
            encoding="utf-8", errors="replace"
        )
    except OSError:
        return False


def main() -> int:
    # A root argument so the control can point this at a planted tree; the real
    # run takes the repo it lives in.
    root = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
    reuse = root / "REUSE.toml"
    if not reuse.is_file():
        print("check-image-licensing: NOTHING WAS READ - no REUSE.toml", file=sys.stderr)
        return 2

    images = tracked_images(root)
    if not images:
        print("check-image-licensing: NOTHING WAS READ - no tracked images", file=sys.stderr)
        return 2

    parsed = blocks(reuse)
    if not parsed:
        print("check-image-licensing: NOTHING WAS READ - no annotation blocks", file=sys.stderr)
        return 2

    # The recorded-unverified set is about THIS repo, so a fixture tree neither
    # excuses anything nor is scolded for missing directories it never had.
    unverified = UNVERIFIED if len(sys.argv) <= 1 else {}

    unnamed: list[str] = []
    excused = 0
    for path in images:
        if any(path.startswith(prefix + "/") for prefix in unverified):
            excused += 1
            continue
        if carries_own_header(root, path) or names_it(path, parsed):
            continue
        unnamed.append(path)

    stale = [p for p in unverified if not any(i.startswith(p + "/") for i in images)]

    if unnamed or stale:
        print("artwork the tree declares only by the catch-all:\n", file=sys.stderr)
        for path in unnamed:
            print(f"  - {path}", file=sys.stderr)
        if unnamed:
            print(
                "\n  Add a REUSE.toml block naming these, or list the directory in "
                "UNVERIFIED with what you found when you looked.",
                file=sys.stderr,
            )
        for path in stale:
            print(f"  - {path} is listed as unverified but holds no image any more", file=sys.stderr)
        return 1

    print(
        f"check-image-licensing: {len(images)} tracked image(s); "
        f"{len(images) - excused} named by a block about them, "
        f"{excused} under {len(unverified)} recorded unverified director(y/ies)."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
