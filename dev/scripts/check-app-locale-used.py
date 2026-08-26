#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""A surface must format with the APP's language, not the browser's.

`toLocaleString()` looks like the locale-aware thing to call. It has "locale" in
the name, it really does behave differently on a German machine, and it is
wrong for an app that carries its own language setting: with no first argument
it asks the BROWSER, so a page set to German writes an English date whenever the
environment happens to be English. That is harder to spot than a hardcoded
string, because the naive reading is that it was handled.

Four were live on 26 August and the two in the coder's lane are fixed: the
viewers app wrote a file's modified date with no locale at all, and the Settings
notifications page passed `undefined` deliberately - on the one line of that page
that says WHEN Do-Not-Disturb ends, which is the part a person reads to decide
whether to wait.

WHAT IS MATCHED. `toLocaleString` / `toLocaleDateString` / `toLocaleTimeString`
with no first argument or an explicit `undefined`, and the `Intl` formatters the
same way. NOT `toLocaleLowerCase` / `toLocaleUpperCase`: those are about letter
casing rules, a different question with a different right answer.

WHAT IS NOT CHECKED, and it is the bigger half: passing SOMETHING is not passing
the right thing. A call handed a variable named `locale` could be reading any
store. This catches the spelling that is always wrong, which is worth a check
precisely because it reads as correct.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

SKIP = {"node_modules", ".svelte-kit", "build", "dist", "target", ".git"}
TREES = ("apps", "sdk")

BARE_METHOD = re.compile(r"\.toLocale(?:Date|Time|)String\(\s*(?:undefined\s*[,)]|\))")
BARE_INTL = re.compile(
    r"\bIntl\.(?:NumberFormat|DateTimeFormat|RelativeTimeFormat|ListFormat)"
    r"\(\s*(?:undefined\s*[,)]|\))"
)

# Calls left as they are, with the reason. Other lanes' files: the rule is the
# same for them, the edit is not the coder's to make. The list may SHRINK and may
# not grow - a new one is a fresh surface written in the browser's language.
ACKNOWLEDGED: dict[str, str] = {
    "apps/harness/src/lib/time.ts": "arlen-ui's app; a relative-time helper whose "
    "fallback writes an absolute date",
    "apps/harness/src/lib/components/transparency/CostSection.svelte": "arlen-ui's app; "
    "the line also builds untranslated English prose around the number "
    "(`... tokens used so far`), so the locale is the smaller half of that fix",
    "apps/store/src/routes/app/[id]/+page.svelte": "arlen-ui's live store work; an "
    "install count interpolated into a translated sentence",
}


def sources(root: Path):
    for tree in TREES:
        base = root / tree
        if not base.is_dir():
            continue
        for f in list(base.rglob("*.ts")) + list(base.rglob("*.svelte")):
            if not set(f.parts) & SKIP:
                yield f


def main() -> int:
    root = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else ROOT
    findings: list[str] = []
    known: list[str] = []
    seen_files: set[str] = set()
    scanned = 0

    for f in sorted(sources(root)):
        scanned += 1
        text = f.read_text(encoding="utf-8", errors="replace")
        rel = str(f.relative_to(root))
        for rx in (BARE_METHOD, BARE_INTL):
            for m in rx.finditer(text):
                line = text[: m.start()].count("\n") + 1
                seen_files.add(rel)
                text_out = (
                    f"{rel}:{line}: formats with no locale, so it asks the browser "
                    f"rather than the app. Pass the app's `locale`."
                )
                if rel in ACKNOWLEDGED:
                    known.append(f"{text_out}\n      reason: {ACKNOWLEDGED[rel]}")
                else:
                    findings.append(text_out)

    if not scanned:
        print(f"NOTHING WAS READ: no frontend sources under {root}", file=sys.stderr)
        return 2

    # Only against the real tree: this list describes THIS repo, so in a fixture
    # tree every entry reads as stale. The two sibling checks learned that the
    # same evening, one of them twice.
    if root == ROOT:
        for rel in sorted(set(ACKNOWLEDGED) - seen_files):
            findings.append(
                f"`{rel}` is acknowledged as formatting without a locale and no longer "
                f"does. Drop the entry."
            )

    print(
        f"{scanned} frontend source(s); {len(known)} call(s) left to another lane, "
        f"each with its reason"
    )
    if known:
        print("\nformatting in the browser's language, acknowledged:\n")
        for k in known:
            print(f"  - {k}")
    if findings:
        print("\na surface writing in a language its reader did not choose:\n")
        for f_ in findings:
            print(f"  - {f_}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
