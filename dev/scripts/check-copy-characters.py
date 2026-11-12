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

THE KIT IS SCANNED NOW, and it was not until 16 September. This file used to
exclude `sdk/ui-kit` and name the one string it was letting through: "Arlen OS
- {$version}", the About line every app shows. The reasoning was ownership - the
copy law is the harness's, and extending it into another lane's file is that
lane's call. The planner ruled on that string on 15 September, which answers the
ownership question the exclusion was waiting on, so the scan covers the kit and
the string is fixed.

THE PRODUCT IS CALLED ARLEN. Not "Arlen OS", which is the other half of the same
finding and the half nothing caught: the middot was reported by this check for
weeks while the name beside it was not. One catalogue string reaches every app's
About dialog, so a name that is wrong there is wrong fifteen times. Matched
case-insensitively and with any whitespace between the words, because the way a
wrong name comes back is somebody typing it from memory.
"""

try:
    import tracked_walk  # noqa: F401  the walk below reads what git knows about
except ModuleNotFoundError:  # a control's copy of this gate, run away from the module
    pass
import pathlib
import re
import subprocess
import sys

# A root may be given, so a control can run this over a fixture instead of over
# the tree. Written after the invoke gate's control edited a real catalogue in
# place to test itself: `.githooks/pre-commit` says at its top that "the gates run
# concurrently", so a control that mutates tracked files is visible to every
# neighbour for as long as it takes, and three unrelated checks went red.
ROOT = pathlib.Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else pathlib.Path(__file__).resolve().parents[2]

M_DASH = "—"
SEPARATOR = re.compile(r"\s·\s")
# The product's name, wrongly. `\b` on the end so "Arlen OSS" or a German word
# starting with OS is not a finding.
PRODUCT_NAME = re.compile(r"\bArlen\s+OS\b", re.I)

# What the helper is called before a person names it: "your assistant", lower
# case (design-system.md section 6.16). Three apps said three other things, and
# each of these is one of them. Read on a catalogue VALUE only, so a comment
# that discusses the agent is not a finding. "the background agent" is a
# different thing and stays.
VALUE = re.compile(r'^\s*"[^"]+":\s*"(.*)"\s*,?\s*$')
HELPER_NAMES = [
    (re.compile(r"\bAsk Arlen\b|\bArlen (fragen|bitten)\b"), "Arlen is the system, not the helper"),
    (re.compile(r"\b[Tt]he (?!background )(agent|assistant)\b"), "the helper is \"your assistant\""),
    (re.compile(r"\b([Dd]e[rnms]|beim|im) (Agent|Assistent)(en)?\b"), "the helper is \"dein Assistent\""),
    (re.compile(r"\b[Yy]our Assistant\b|\b[Yy]our \{\$(name|assistant|companion)\b"), "lower case, and a name replaces the whole phrase"),
]


def catalogues() -> list[pathlib.Path]:
    """Every message catalogue under ROOT, the kit's included.

    Asks git when ROOT is a repository, so an untracked scratch file cannot fail a
    run; walks the directory when it is not, which is the case for a fixture tree.
    """
    try:
        out = subprocess.run(
            ["git", "ls-files"], cwd=ROOT, capture_output=True, text=True, check=True
        ).stdout.split()
        found = [ROOT / f for f in out if "/i18n/messages" in f and f.endswith(".ts")]
    except (subprocess.CalledProcessError, FileNotFoundError):
        found = [p for p in ROOT.rglob("*/i18n/messages*.ts")]
    return sorted(found)


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
        if PRODUCT_NAME.search(line):
            found.append(f"{where}: the product is called Arlen, never \"Arlen OS\".\n    {body}")
        value = VALUE.match(line)
        if value:
            for pattern, why in HELPER_NAMES:
                if pattern.search(value.group(1)):
                    found.append(f"{where}: {why} (design-system.md 6.16).\n    {body}")
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
        print(f"\n{len(bad)} finding(s) across {len(files)} catalogues")
        return 1
    print(f"check-copy-characters: {len(files)} catalogues clean")
    return 0


if __name__ == "__main__":
    sys.exit(main())
