#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that no catalogue defines the same message id twice in one locale.

A duplicate is always wrong and never says so. JavaScript keeps the LAST value
written, so the earlier line is dead: it is read by nobody, it survives review
because both lines look right on their own, and the one that wins is decided by
file order rather than by anybody.

It is here because I wrote three of them in a single edit on 23 August, adding
tile keys that already existed further down the same catalogue - the literals had
appeared in the tiles' doc comments and I read them as findings. A grep for
duplicates before committing caught it. Nothing else would have: `check-catalogs`
compiles every message and a duplicate compiles, `check-message-keys` asks whether
a key is DEFINED and a duplicate is defined twice.

What it checks: every `*/src/lib/i18n/*.ts` catalogue, per locale block, for an id
that appears more than once.

What it cannot check: whether the surviving value is the right one. Two lines with
the same id and different text are the case that hurts most, and this only says
they are there.

**AND IT ONLY LOOKED INSIDE ONE FILE, which is where the real one was hiding.**
Settings splits its catalogue across `messages.a.ts` and `messages.b.ts` and merges
them with `{...a.en, ...b.en}`, so a key in both is a duplicate with the same
consequence - b wins, a is dead - and this saw neither line twice. Eight of them
had accumulated. One had teeth: `s.revert.keep` was the display revert dialog's
TITLE in one file and its BUTTON in the other, and the button's text takes a
`{$seconds}` parameter, so the heading - called without one - rendered
`Aenderungen behalten ({$seconds} s)` at the top of the modal. Found by
photographing that modal on 8 September, not by reading either file.

So the second pass groups by (app, locale) across every catalogue file the app
merges, and reports a key that more than one of them defines.
"""

import collections
import pathlib
import re
import sys

ROOT = (
    pathlib.Path(sys.argv[1]).resolve()
    if len(sys.argv) > 1
    else pathlib.Path(__file__).resolve().parents[2]
)

LOCALE = re.compile(r"\n  ([a-z]{2}(?:-[A-Z]{2})?): \{")
KEY = re.compile(r'^\s*"([^"]+)":', re.M)


def catalogues() -> list[pathlib.Path]:
    """Every catalogue in the tree, apps and daemon frontends and the kit."""
    out: list[pathlib.Path] = []
    for pattern in ("apps/*/src/lib/i18n/*.ts", "daemons/*/*/src/lib/i18n/*.ts",
                    "sdk/ui-kit/src/lib/i18n/*.ts"):
        out += sorted(ROOT.glob(pattern))
    return [p for p in out if "node_modules" not in p.parts and not p.name.endswith(".test.ts")]


def locale_blocks(text: str) -> list[tuple[str, str]]:
    """(locale, body) for each `xx: { … }` block, by brace matching."""
    blocks = []
    for m in LOCALE.finditer(text):
        depth, i = 1, m.end()
        while i < len(text) and depth:
            if text[i] == "{":
                depth += 1
            elif text[i] == "}":
                depth -= 1
            i += 1
        blocks.append((m.group(1), text[m.end() : i]))
    return blocks


def main() -> int:
    files = catalogues()
    if not files:
        print(f"NOTHING WAS READ: no catalogue under {ROOT}", file=sys.stderr)
        return 2

    findings: list[str] = []
    checked = 0
    # (app directory, locale) -> key -> the files defining it. An app may split
    # its catalogue over several files and merge them; a key in two of those is
    # the same defect as a key twice in one, and the merge order decides it.
    across: dict[tuple[str, str], dict[str, list[str]]] = collections.defaultdict(
        lambda: collections.defaultdict(list)
    )
    for path in files:
        text = path.read_text(encoding="utf-8", errors="replace")
        app = str(path.parent.relative_to(ROOT))
        for locale, body in locale_blocks(text):
            keys = KEY.findall(body)
            checked += len(keys)
            for key in set(keys):
                across[(app, locale)][key].append(path.name)
            for key, count in sorted(collections.Counter(keys).items()):
                if count > 1:
                    findings.append(
                        f"{path.relative_to(ROOT)}: `{key}` is defined {count} times "
                        f"in `{locale}`. The last one wins and the others are dead, "
                        f"so which sentence a reader gets is decided by line order."
                    )

    for (app, locale), keys in sorted(across.items()):
        for key, names in sorted(keys.items()):
            if len(names) > 1:
                findings.append(
                    f"{app}: `{key}` is defined in `{locale}` by "
                    f"{', '.join(sorted(names))}. They are merged in order, so the "
                    f"later file wins and the earlier definition is dead."
                )

    if not checked:
        print("NOTHING WAS READ: no message id in any catalogue", file=sys.stderr)
        return 2

    if findings:
        print("a message id defined more than once:")
        for f in findings:
            print(f"  - {f}")
        return 1

    print(
        f"{checked} message id(s) across {len(files)} catalogue(s), none defined "
        f"twice in one locale and none defined by two of an app's files. Says "
        f"nothing about whether the text is right - only that no line is silently "
        f"dead."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
