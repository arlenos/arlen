#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A message that takes a parameter must not be called without one.

MessageFormat 2 formats an unsupplied `{$name}` by printing the placeholder, in
bidi isolates, exactly where the value should have been. Nothing throws and
nothing logs; the sentence simply arrives with its hole showing.

Found on 8 September by photographing the display revert modal, which headed
itself:

    Aenderungen behalten (<{$seconds}> s)

The cause there was one id doing two jobs - `s.revert.keep` was the dialog's
title in one catalogue file and its button in another, and the button's text
takes `{$seconds}` - and `check-catalog-duplicates` covers that cause now. This
covers the SHAPE, which arrives without any duplication: somebody adds a
parameter to an existing message and misses one of its call sites, and every
other caller keeps working.

What it checks: a call with a literal id and NO second argument - `$t("k")`,
`t("k")`, `$kt("k")` - against the text that id resolves to. If the text carries
a `{$…}` in any locale, the call is a hole on screen.

What it cannot check, and both are deliberate:

  * a call whose id is a VARIABLE. `{$t(line.provenance.id, line.provenance.params)}`
    is the normal shape for a message chosen from data, and no static reader can
    say which id it lands on. Those pass the params along anyway, which is why
    the shape exists.
  * whether the parameters passed are the RIGHT ones. A call with a second
    argument is accepted whatever it holds; MessageFormat prints the placeholder
    for a missing key inside an object just the same. That needs the render, and
    the render is what found this one.
"""

import os
import pathlib
import re
import sys

ROOT = (
    pathlib.Path(sys.argv[1]).resolve()
    if len(sys.argv) > 1
    else pathlib.Path(__file__).resolve().parents[2]
)

LOCALE = re.compile(r"\n  ([a-z]{2}(?:-[A-Z]{2})?): \{")
# `"id": "text"`, with the text captured including its escapes so a `\"` inside
# does not end it early.
ENTRY = re.compile(r'^\s*"([^"]+)":\s*("(?:[^"\\]|\\.)*")', re.M)
# `{$name`, which covers `{$n}`, `{$n :number}` and `{$n, plural, …}` alike. The
# closing brace is not required: the forms differ after the name, never before.
VAR = re.compile(r"\{\s*\$[A-Za-z_][A-Za-z0-9_]*\b")
# A translator call with a literal id and nothing after it. `$t`, `t` and the
# kit's `$kt` all reach a catalogue the same way.
CALL = re.compile(r'(?<![A-Za-z0-9_$])\$?k?t\(\s*"([^"]+)"\s*\)')

SKIP_DIRS = {"target", "node_modules", ".git", ".svelte-kit", "build", "dist", ".vite"}


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


def parameterised(root: pathlib.Path) -> dict[str, set[str]]:
    """Every message id whose text takes a parameter, to the locales it does in.

    Keyed by id alone rather than per app. An id is global enough in practice -
    every app prefixes its own - and a per-app map would miss the kit's ids,
    which apps call through `$kt`.
    """
    out: dict[str, set[str]] = {}
    for pattern in (
        "apps/*/src/lib/i18n/*.ts",
        "daemons/*/*/src/lib/i18n/*.ts",
        "sdk/ui-kit/src/lib/i18n/*.ts",
    ):
        for path in sorted(root.glob(pattern)):
            if "node_modules" in path.parts or path.name.endswith(".test.ts"):
                continue
            text = path.read_text(encoding="utf-8", errors="replace")
            for locale, body in locale_blocks(text):
                for m in ENTRY.finditer(body):
                    if VAR.search(m.group(2)):
                        out.setdefault(m.group(1), set()).add(locale)
    return out


def callers(root: pathlib.Path):
    """(path, line, id) for every literal translator call taking no parameters."""
    for base, dirs, files in os.walk(root / "apps"):
        dirs[:] = [d for d in dirs if d not in SKIP_DIRS]
        for name in files:
            if not name.endswith((".svelte", ".ts")):
                continue
            path = pathlib.Path(base) / name
            if "i18n" in path.parts or name.endswith(".test.ts"):
                continue
            text = path.read_text(encoding="utf-8", errors="replace")
            for i, line in enumerate(text.splitlines(), 1):
                for m in CALL.finditer(line):
                    yield path, i, m.group(1)


def main() -> int:
    takes = parameterised(ROOT)
    if not takes:
        print(
            f"NOTHING WAS READ: no parameterised message under {ROOT}",
            file=sys.stderr,
        )
        return 2

    findings = []
    checked = 0
    for path, line, key in callers(ROOT):
        checked += 1
        if key in takes:
            findings.append(
                f"{path.relative_to(ROOT)}:{line}: `{key}` takes a parameter in "
                f"{', '.join(sorted(takes[key]))} and is called with none, so the "
                f"placeholder renders where the value belongs"
            )

    if findings:
        print("a message called without the parameter it takes:")
        for f in findings:
            print(f"  - {f}")
        print("\n  Pass the parameter, or give the parameterless caller its own id.")
        return 1

    print(
        f"{checked} literal translator call(s) with no parameters, none of them "
        f"naming one of the {len(takes)} message(s) that take one. A call whose id "
        f"is a variable is not read here, and neither is whether the parameters a "
        f"call DOES pass are the right ones."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
