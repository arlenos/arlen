#!/usr/bin/env python3
"""A character the copy law bans must not reach a message catalogue.

WHY A CATALOGUE AND NOT THE TREE. There are 1310 m-dashes in this repository and
none of them reaches a screen: they are all in comments and module docs, where
they are a style matter. A message catalogue is the one place where every value is
user-facing by construction, so a rule about what a person reads can be enforced
there exactly, with no judgement about whether a given string is copy. That keeps
the gate quiet enough to stay believed.

WHAT IT BANS. Two characters, and each is read off a rule that already exists
rather than invented here:

  * the m-dash, which is Tim's standing rule for every string this project writes;
  * the middot used as a SEPARATOR, from the copy law in
    `apps/harness/src/lib/display.ts` ("no em-dashes, no middot separators"). The
    space on both sides is what distinguishes decoration from spelling, so a
    middot inside a word is left alone.

WHAT IT DOES NOT BAN, recorded because I got it wrong first. The single-glyph
ellipsis is NOT banned. The first cut of this gate refused it and found 61,
nearly all of the form "Rename...", "Open with...", "Loading...". That is the
platform convention for "this opens a dialog", it is what every desktop does, and
no rule in this tree asks for it to change. I had read the copy law's two bans and
added a third from memory - which is how a gate starts producing noise and stops
being read.

NOT SCANNED, deliberately: `sdk/ui-kit` is another lane's. Its kit catalogue
carries one middot separator today ("Arlen OS - {$version}" in an About line);
that is reported to arlen-ui rather than edited here, since the copy law is the
harness's own and extending it into their file is their call, not mine.
"""

import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]

M_DASH = "—"
SEPARATOR = re.compile(r"\s·\s")


def catalogues() -> list[pathlib.Path]:
    """Every app message catalogue this repository tracks, kit excluded."""
    out = subprocess.run(
        ["git", "ls-files"], cwd=ROOT, capture_output=True, text=True, check=True
    ).stdout.split()
    return [
        ROOT / f
        for f in out
        if "/i18n/messages" in f and f.endswith(".ts") and not f.startswith("sdk/ui-kit/")
    ]


def findings(path: pathlib.Path) -> list[str]:
    """The banned characters in one catalogue, as lines a reader can act on."""
    found = []
    for n, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        where = f"{path.relative_to(ROOT)}:{n}"
        body = line.strip()[:120]
        if M_DASH in line:
            found.append(f"{where}: an m-dash. Use a comma, a semicolon or a second sentence.\n    {body}")
        if SEPARATOR.search(line):
            found.append(f"{where}: a middot used as a separator. Use a comma or a new sentence.\n    {body}")
    return found


def main() -> int:
    """Refuse a catalogue carrying a character the copy law bans."""
    files = catalogues()
    if not files:
        print("check-copy-characters: no message catalogue found, which is itself wrong")
        return 1
    bad = [f for p in files for f in findings(p)]
    for f in bad:
        print(f)
    if bad:
        print(f"\n{len(bad)} banned character(s) across {len(files)} catalogues")
        return 1
    print(f"check-copy-characters: {len(files)} catalogues clean")
    return 0


if __name__ == "__main__":
    sys.exit(main())
