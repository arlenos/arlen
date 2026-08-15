#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""A file an image build script writes must land where the script prepared.

Two failures, one family, both from moving a path in one place and leaving it in
another. Both happened on 15 Aug within a minute of each other, in the same edit:

  * the kg probe unit moved from `systemd/system` to `systemd/user`, and the
    `mkdir -p` above it kept naming `system`. `cat >` does not create parents, so
    the build died with `Directory nonexistent`;
  * the `chmod 644` below it also kept naming `system`, so even with the mkdir
    fixed it would have died one line later on a file that was never there.

WHY A GATE RATHER THAN CARE. These scripts run inside mkosi, after the Debian
rootfs and ten-odd minutes of cargo and npm. The failure arrives at the very end
of a long build, and `mkosi --force` has already deleted the previous image by
then - the build script's rename trap is what puts it back. So the cost of the
typo is a quarter hour and a restored-from-backup image, for something a string
comparison settles in a second.

It also catches the version that does NOT fail loudly. A `chmod` on the wrong
path fails, but a write into a directory that some OTHER script happened to
create succeeds and puts the file somewhere nobody looks for it - which shows up
much later as a unit that is simply never started, with no error anywhere.

WHAT IT DOES NOT CHECK. Only literal paths under `$DESTDIR`. A path built from a
shell variable is skipped rather than guessed at, because guessing produces
confident nonsense about paths that do not exist (the failure mode that made
`check-emitters-declared.py` refuse to read these same build steps for its
shipping question). Every skip is printed, so an unreadable script is visible
rather than silently counted as clean.
"""

import re
import sys
from pathlib import Path

OWN_TREE = len(sys.argv) <= 1
REPO = Path(__file__).resolve().parents[2] if OWN_TREE else Path(sys.argv[1]).resolve()

STEPS = "dev/mkosi/mkosi.build.d"

# `cat > "$DESTDIR/a/b/c"`, and the plain redirect form. A symlink counts too:
# `ln -sf target "$DESTDIR/link"` creates the LINK, so it needs the link's parent
# and nothing more. Reading it the other way round - as something acting on a
# file that must already exist - reported all six of the tree's `/usr/bin`
# symlinks as defects on this gate's first run.
WRITE = re.compile(
    r'(?:cat|tee|printf|echo)\s[^\n]*?>>?\s*"\$DESTDIR/(?P<path>[^"$]+)"'
    r'|ln\s+-s[a-z]*\s+[^\n]*?"\$DESTDIR/(?P<link>[^"$]+)"'
)
# `install -Dm755 src "$DESTDIR/a/b/c"` - -D makes the parents itself.
INSTALL_D = re.compile(r'install\s+[^\n]*-D[^\n]*"\$DESTDIR/(?P<path>[^"$]+)"')
# `install -m644 src "$DESTDIR/a/b/c"` without -D needs the directory to exist.
# `-d` is excluded here and read as a mkdir below, which is what it is.
INSTALL_PLAIN = re.compile(
    r'install\s+(?![^\n]*-D)[^\n]*"\$DESTDIR/(?P<path>[^"$]+)"'
)
# `install -d` makes a directory. The flag can sit anywhere after the command,
# including immediately after it, which an earlier `\s-d` pattern missed - so
# every `install -d -m755` line read as a file write into its own parent.
MKDIR = re.compile(r'(?:mkdir\s+-p|install\s+(?:[^\n]*?\s)?-d\b)\s+(?P<args>[^\n]+)')
MKDIR_ARG = re.compile(r'"\$DESTDIR/(?P<path>[^"$]+)"')
# Only the commands that genuinely need the file to be there already.
TOUCHES = re.compile(r'(?P<cmd>chmod|chown)\s+[^\n]*?"\$DESTDIR/(?P<path>[^"$]+)"')


def parent_of(path: str) -> str:
    return str(Path(path).parent)


def dirs_made_by(line_no: int, made: list[tuple[int, str]]) -> set[str]:
    """Every directory that exists by the time `line_no` runs."""
    out: set[str] = set()
    for at, d in made:
        if at >= line_no:
            continue
        # `mkdir -p a/b/c` makes a, a/b and a/b/c.
        p = Path(d)
        out.add(str(p))
        out.update(str(a) for a in p.parents)
    return out


def main() -> int:
    steps = sorted((REPO / STEPS).glob("*.chroot"))
    if not steps:
        print(f"NOTHING WAS READ: no build step under {REPO}/{STEPS}", file=sys.stderr)
        return 2

    problems: list[str] = []
    skipped: list[str] = []
    checked = 0

    for step in steps:
        rel = step.relative_to(REPO)
        text = step.read_text(encoding="utf-8", errors="replace")
        lines = text.splitlines()

        # Join backslash continuations FIRST, keeping the line number of the
        # physical line the command starts on.
        #
        # Every pattern here uses `[^\n]*?` between the command and its
        # `$DESTDIR` path, so a command split across two physical lines matched
        # nothing at all - it was not flagged, it was never examined. That is the
        # worst failure mode a gate has: it reported OK on a step whose `ln` died
        # at `No such file or directory` on the very next build, and the two
        # symlinks in the tree written this way had simply never been checked.
        joined: list[tuple[int, str]] = []
        buf, start = "", 0
        for n, raw in enumerate(lines, start=1):
            if not buf:
                start = n
            if raw.rstrip().endswith("\\"):
                buf += raw.rstrip()[:-1] + " "
                continue
            joined.append((start, buf + raw))
            buf = ""
        if buf:
            joined.append((start, buf))
        lines = [text for _, text in joined]
        line_no = {idx: n for idx, (n, _) in enumerate(joined, start=1)}

        made: list[tuple[int, str]] = []
        written: dict[str, int] = {}

        for i, line in enumerate(lines, start=1):
            if line.lstrip().startswith("#"):
                continue
            if m := MKDIR.search(line):
                for a in MKDIR_ARG.finditer(m.group("args")):
                    made.append((i, a.group("path")))
                if "$DESTDIR" in m.group("args") and not MKDIR_ARG.search(m.group("args")):
                    skipped.append(f"{rel}:{line_no.get(i, i)}: mkdir path is not a literal, not checked")
            for m in INSTALL_D.finditer(line):
                # -D makes the parents, so the directory exists from here on.
                made.append((i, parent_of(m.group("path"))))
                written[m.group("path")] = i

        for i, line in enumerate(lines, start=1):
            if line.lstrip().startswith("#"):
                continue

            for m in list(WRITE.finditer(line)) + list(INSTALL_PLAIN.finditer(line)):
                path = m.group("path") or m.groupdict().get("link")
                # A directory-making line is not a write, whichever spelling.
                if not path or INSTALL_D.search(line) or MKDIR.search(line):
                    continue
                checked += 1
                written[path] = i
                parent = parent_of(path)
                if parent not in dirs_made_by(i, made):
                    problems.append(
                        f"{rel}:{line_no.get(i, i)}: writes $DESTDIR/{path}, but nothing above it "
                        f"creates $DESTDIR/{parent}.\n"
                        f"    `cat >` does not make parents, so this dies with "
                        f"`Directory nonexistent` at the end of a full image build - "
                        f"or worse, succeeds because another step happened to make "
                        f"the directory, and the file lands where nobody looks."
                    )

            for m in TOUCHES.finditer(line):
                path, cmd = m.group("path"), m.group("cmd")
                checked += 1
                if path not in written or written[path] > i:
                    problems.append(
                        f"{rel}:{i}: {cmd} on $DESTDIR/{path}, which nothing above "
                        f"it writes.\n"
                        f"    Usually the file moved and this line kept the old "
                        f"path. It fails the build at the last step, after every "
                        f"compile has been paid for."
                    )

    if skipped:
        print("not checked (the path is not a literal):")
        for s in skipped:
            print(f"  {s}")
        print()

    if problems:
        print("a build step writes somewhere it did not prepare:")
        for p in problems:
            print(f"  {p}")
        return 1

    print(f"OK: {checked} image write(s), each into a directory its step creates")
    return 0


if __name__ == "__main__":
    sys.exit(main())
