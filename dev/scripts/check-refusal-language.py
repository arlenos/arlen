# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a translated sentence is not completed with a raw error string.

Written on 25 August after making it three times in one night, twice in code I had
just fixed the same defect in.

THE SHAPE. A catalog entry reads `"Dieses Dokument konnte nicht geöffnet werden:
{$reason}"` and the caller fills `reason` with `String(e)`. What a German reader
then meets is half a translated sentence and half whatever the backend happened to
format - `this file could not be read as a PDF: <parser detail>`, or
`spawn bwrap: No such file or directory (os error 2)`, errno and all. The window
did its job, the value arriving in it did not.

The fix is always the same and is not "translate the error". A backend answers a
routine failure with a TOKEN and the window writes the whole sentence, so there is
one place where a language is chosen. That is how the locked-PDF case, the sound
resolver and the bottle daemon already work.

WHAT COUNTS. A translate call whose argument object is filled from `String(e)` -
that is, from a stringified exception. Nothing else: a `{$path}` filled with a
path, a `{$count}` filled with a number and a `{$name}` filled with a filename are
all data the sentence is about, and are fine.

ACKNOWLEDGED. Quick Settings guards both of its calls with `readsAsInternal`, so an
error that names an internal falls to a plain sentence and only a service's own
readable words are interpolated. That is a considered trade by its author rather
than an oversight, and it is listed here rather than exempted by helper name -
a gate that waives a shape whenever a particular function appears beside it is
waiving the shape. The residual is real and belongs to whoever owns that page: the
predicate reads ENGLISH text, so a readable English clause still reaches a German
reader as English.

WHAT IT CANNOT SEE. A raw string that reaches the argument through a variable
(`const msg = String(e)` and then `{ reason: msg }`). Widening to that means
tracking assignments, and a check that guesses at dataflow reports confident
nonsense; this catches the shape as it is actually written. It also says nothing
about a raw error assigned straight into state and rendered without a catalog at
all - `check-unrendered-error` is the gate for that side.

Run: dev/scripts/check-refusal-language.py [root]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

#: Where a frontend lives. The SDK's shared components are held to the same rule.
SOURCES = ("apps/*/src", "sdk/ui-kit/src")

#: `$t("key", { ... String(e) ... })` and `get(t)("key", { ... })`, on one line.
#: The catalog key is required, so this cannot fire on an unrelated call taking an
#: object; the stringified exception is required, so it cannot fire on real data.
CALL = re.compile(
    r"""\$?t\)?\(\s*["'](?P<key>[a-zA-Z][\w.]*)["']\s*,\s*\{[^}]*String\(\s*(?:e|err|error|ex)\s*\)""",
)


#: Calls argued for rather than fixed, as `path:line` with the reason. A stale
#: entry is worse than none, so an entry whose FILE is present and whose line no
#: longer matches is an error: it would otherwise excuse a line somebody has since
#: rewritten. An entry whose file is absent says nothing, because then this root is
#: not the repository and there is nothing to be stale about.
ACKNOWLEDGED = {
    "apps/settings/src/routes/appearance/quicksettings/+page.svelte:292": (
        "guarded by `readsAsInternal`: an internal-looking error falls to "
        "`s.qs.saveFailedPlain` and only a readable service message is interpolated"
    ),
    "apps/settings/src/routes/appearance/quicksettings/+page.svelte:385": (
        "guarded by `readsAsInternal`: same trade on the reset path"
    ),
}


def main() -> int:
    files: list[Path] = []
    for pattern in SOURCES:
        for base in ROOT.glob(pattern):
            files.extend(p for p in base.rglob("*.ts") if "node_modules" not in p.parts)
            files.extend(p for p in base.rglob("*.svelte") if "node_modules" not in p.parts)

    if not files:
        print(
            "NOTHING WAS READ: no frontend sources found, so no sentence was checked",
            file=sys.stderr,
        )
        return 2

    problems: list[str] = []
    seen_acknowledged: set[str] = set()
    for path in sorted(files):
        for n, line in enumerate(path.read_text(encoding="utf-8", errors="replace").splitlines(), 1):
            match = CALL.search(line)
            if match:
                where = f"{path.relative_to(ROOT)}:{n}"
                if where in ACKNOWLEDGED:
                    seen_acknowledged.add(where)
                    continue
                problems.append(
                    f"{where}: `{match.group('key')}` is completed with a "
                    f"stringified error, so half the sentence is in the reader's language and "
                    f"half is whatever the backend formatted"
                )

    if problems:
        print("translated sentences finished by a raw error:\n")
        for p in problems:
            print(f"  - {p}")
        print(
            "\nHave the backend answer with a token and write the whole sentence here.\n"
            "One place chooses the language; the detail belongs in the log."
        )
        return 1

    stale = sorted(
        where
        for where in set(ACKNOWLEDGED) - seen_acknowledged
        if (ROOT / where.rsplit(":", 1)[0]).is_file()
    )
    if stale:
        print("acknowledged calls that no longer exist:\n")
        for where in stale:
            print(f"  - {where}")
        print(
            "\nThe line moved or was rewritten. Remove the entry or point it at the new\n"
            "line: a stale excuse says a known problem is known when it is not there."
        )
        return 1

    note = f", {len(ACKNOWLEDGED)} acknowledged" if ACKNOWLEDGED else ""
    print(
        f"check-refusal-language: {len(files)} frontend source(s){note}; no translated sentence "
        f"is finished with a stringified error."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
