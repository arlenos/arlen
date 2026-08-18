# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a profile's description does not claim a grant the profile lacks.

Written on 17 August after making the defect. Narrowing `org.x.Warpinator` to the
file portal, I removed its `[filesystem]` table and appended the reason, and left
the first paragraph saying "It gets network and Downloads" - a profile
contradicting itself in five lines, in the same hour as a sweep removing exactly
that from other people's files.

It matters more than a tidy comment. These descriptions are the only statement of
INTENT a profile carries; the grants are the mechanism. When they disagree, the
next reader either widens the grant back to match the prose or trusts a sentence
the machine never honoured, and both are how a permission corpus rots.

Deliberately narrow. It reads only the LEADING comment paragraph, only sentences
containing "gets", only capitalised directory names, and skips negations - four
restrictions, each of which came from a real false positive in this corpus rather
than from imagination:

  1. Later paragraphs are notes explaining a change, and they QUOTE the discarded
     reasoning: "this held the whole home tree on the reasoning that ... so it
     gets home filesystem access". A quoted claim is not a claim.
  2. The first cut required the directory immediately after "gets", and so did not
     find "gets network and Downloads" - the very sentence it was written for.
     Sentence-scoped now.
  3. `showtime` says "so it gets Videos and Music - not Home". Negated mentions
     are the corpus telling you what an app does NOT get.
  4. "it downloads themes, so it gets network" and "music streaming" are a verb
     and a common noun. The corpus writes the DIRECTORY capitalised, so matching
     case-sensitively separates them; case-insensitive matching produced eight
     false positives and would have trained people to reword around the check.

A heuristic over prose earns its place only by being right on the whole corpus:
this one reports nothing on 2319 profiles and finds the planted defect. A
different one, flagging any directory word a profile mentions, was measured at
three findings and three false positives, and was NOT gated for that reason.
"""

import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
PROFILES = ROOT / "sdk/permissions/profiles"

#: A capitalised XDG directory, optionally negated. Case-sensitive on purpose.
DIRECTORY = re.compile(
    r"\b(?P<neg>not\s+|no\s+)?(?P<dir>Documents|Downloads|Pictures|Music|Videos|Home)\b"
)

#: The verb that turns a mention into a claim about this profile's own grant.
GETS = re.compile(r"\bgets?\b", re.I)


def leading_paragraph(text: str) -> str:
    """The description, which is the comment block before the first `#` spacer."""
    out: list[str] = []
    for line in text.splitlines():
        if not line.startswith("#") or line.strip() == "#":
            break
        out.append(line.lstrip("# ").rstrip())
    return " ".join(out)


def main() -> int:
    if not PROFILES.is_dir():
        print(f"NOTHING WAS READ: no profiles at {PROFILES}", file=sys.stderr)
        return 2

    findings: list[str] = []
    read = 0

    for path in sorted(PROFILES.glob("*.toml")):
        text = path.read_text(encoding="utf-8")
        try:
            doc = tomllib.loads(text)
        except tomllib.TOMLDecodeError as e:
            print(f"{path.name}: does not parse ({e})", file=sys.stderr)
            return 1
        read += 1
        granted = {k for k, v in doc.get("filesystem", {}).items() if v is True}

        for sentence in re.split(r"(?<=[.;])\s+", leading_paragraph(text)):
            if not GETS.search(sentence):
                continue
            for m in DIRECTORY.finditer(sentence):
                if m.group("neg"):
                    continue
                if m.group("dir").lower() not in granted:
                    findings.append(
                        f"{path.name}: says it gets {m.group('dir')} but grants "
                        f"{sorted(granted) or 'no filesystem'}: {sentence.strip()[:88]}"
                    )

    if not read:
        print("NOTHING WAS READ: no profile parsed", file=sys.stderr)
        return 2

    # The summary is printed only when there is nothing to report. It used to be
    # printed FIRST, unconditionally, so a failing run ended on the words "no
    # description claims a directory its grants do not include" - on stdout,
    # after the findings had gone to stderr. The exit code was right and the last
    # line a reader sees was the opposite of it, which is the shape this whole
    # directory exists to stop.
    if findings:
        print(f"{read} profile(s) checked, {len(findings)} claiming a grant they do not have:\n", file=sys.stderr)
        for f in findings:
            print(f"  - {f}", file=sys.stderr)
        print(
            "\nCorrect the sentence to describe the grant that is there. If the grant "
            "was removed on purpose, the description is the other half of that change.",
            file=sys.stderr,
        )
        return 1

    print(
        f"{read} profile(s) checked: no description claims a directory its grants do "
        f"not include. The description is a profile's only statement of intent, so a "
        f"claim the mechanism does not honour is the one a later reader acts on."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
