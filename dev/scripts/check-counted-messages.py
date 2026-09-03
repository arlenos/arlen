# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a message putting a COUNT before a plural noun selects on it.

The catalogues are MessageFormat 2, and the runtime under them is `messageformat`
v4, so plural selection is available everywhere: `.input {$n :number}` then
`.match $n` then a `one` branch and a `*` branch. The kit's own i18n header states
the rule - "messages are grammatically whole (plurals/gender/word-order via MF2
selectors), never concatenated fragments".

Plenty of messages already do it. The ones that do not are invisible until the
count happens to be one, and then a surface says "1 files", "1 events",
"1 messages in this conversation" or, the one that started this check, "1 of its
paths lead outside it" in a security warning about a Windows app's reach. Nothing
tests with a count of one, because a fixture is written with a plausible number in
it, so these survive every render and every screenshot.

The rule is narrow on purpose: a placeholder whose NAME says it is a count
(`$n`, `$count`, `$total`, ...), directly before a bare plural noun, in a message
with no `.match` in it. Anything looser reports `{$app} wants` and `{$from} bis`,
and a check that cries wolf is one people learn to run with their eyes closed.

THE PLURAL-NOUN RULE READS THE ENGLISH, and cannot do otherwise: German plurals
are `-e`, `-en`, `-er`, `-n`, `-s` and an umlaut, which no suffix test separates
from a singular. English is the catalogue's source of truth, so a message needing
a selector needs one there first - but a translation can still drop it, so the
second rule closes that: every locale's entry for one key agrees about whether it
selects. A key that selects in English and not in German is a German surface that
says "1 Dateien" while the English one is right, which is the drift that survives
longest because the person who can see it is not the person who wrote it.

What this does NOT check: a message that selects on the wrong variable, or one
whose `one` branch is itself ungrammatical. Both need a reader.

Shown to fail before being trusted: the control plants each shape.
"""

import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

# Placeholder names that mean "how many". A message is only judged when the author
# already said, by naming it, that the value is a count.
COUNTS = {"n", "count", "total", "cut", "left", "days", "files", "items", "errors", "num"}

# A count placeholder, whitespace, then a word ending in a plural suffix.
COUNTED = re.compile(r"\{\$(\w+)\}\s+([a-z]+(?:s|es|ies))\b")

# Words that end in `s` and are not plural nouns.
NOT_PLURAL = {"is", "was", "has", "this", "its", "less", "across", "plus", "minus", "us", "yes"}

ONE_LINE = re.compile(r'^\s*"([\w.]+)":\s*"(.*)",?\s*$')

# Messages that are somebody else's to change, and why. The store's frontend is
# arlen-ui's live surface, so its catalogue is theirs; naming them here is the
# difference between a known boundary and an unowned red.
ACKNOWLEDGED = {
    "st.capCount": "apps/store is arlen-ui's surface",
    "st.trust.installs.value": "apps/store is arlen-ui's surface",
    "st.shown": "apps/store is arlen-ui's surface",
    "st.observed.window": "apps/store is arlen-ui's surface",
}


def catalogues() -> list[Path]:
    found = sorted((ROOT / "apps").glob("*/src/lib/i18n/messages*.ts"))
    kit = ROOT / "sdk/ui-kit/src/lib/i18n/messages.kit.ts"
    if kit.is_file():
        found.append(kit)
    return found


def main() -> int:
    files = catalogues()
    if not files:
        print(f"no message catalogues under {ROOT}; the layout moved and this check did not")
        return 1

    problems: list[str] = []
    checked = 0
    # key -> [(file, line, selects)], so the locales of one key can be compared.
    seen: dict[str, list[tuple[str, int, bool]]] = {}
    for path in files:
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            m = ONE_LINE.match(line)
            if not m:
                continue
            key, value = m.groups()
            checked += 1
            seen.setdefault(key, []).append(
                (str(path.relative_to(ROOT)), number, ".match" in value)
            )
            if ".match" in value:
                continue
            for hit in COUNTED.finditer(value):
                if hit.group(1) not in COUNTS or hit.group(2) in NOT_PLURAL:
                    continue
                if key in ACKNOWLEDGED:
                    continue
                problems.append(
                    f"{path.relative_to(ROOT)}:{number} {key} reads "
                    f"“{hit.group(0)}” and selects on nothing, so at one it says "
                    f"“1 {hit.group(2)}”. Give it a selector: "
                    f'".input {{${hit.group(1)} :number}}\\n.match ${hit.group(1)}\\n'
                    f'one {{{{...}}}}\\n*   {{{{...}}}}".'
                )

    for key, entries in sorted(seen.items()):
        if key in ACKNOWLEDGED or len(entries) < 2:
            continue
        selecting = [e for e in entries if e[2]]
        if selecting and len(selecting) != len(entries):
            silent = ", ".join(f"{f}:{n}" for f, n, sel in entries if not sel)
            problems.append(
                f"{key} selects on its count in one locale and not in another "
                f"({silent}). The translated surface then says the plural at one "
                f"while the source language is right."
            )

    print(
        f"{checked} message line(s) across {len(files)} catalogue(s): every counted one "
        f"selects on its count, every key's locales agree about selecting, or it is "
        f"listed as somebody else's ({len(ACKNOWLEDGED)})."
    )
    if problems:
        print("\ncounted messages that read wrong at one:\n")
        for p in problems:
            print(f"  - {p}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
