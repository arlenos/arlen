#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""A sentence a person reads, written in English where the catalogue cannot see it.

THE SHAPE, and it is not the sibling's. `check-untranslated-render.py` looks for a
TAINTED value - something fed by `String(e)` - reaching the markup. This looks for
prose that was simply typed in the wrong place: a literal returned from a function
or assigned to a user-facing name, never passing through `$t`.

Neither the catalogue checks nor that sibling can see it, and the reason is worth
stating because it is why this went unnoticed for so long: **nothing is missing
from the catalogue.** The sentence never asks it. A completeness check compares
the two locales and finds them equal; a render-site scan sees a variable and
cannot know what is in it.

Six were found by hand on 6 September and every one was on a surface somebody uses
daily: the Waypointer's `Enter: Run / Shift+Enter: Terminal` and its three
siblings, written into the DOM by a polling loop, so the launcher answered a
German reader in English; `capabilitySentence()` returning two English sentences
about the agent; the network indicator's `Signal strength {n}%` and a notification
group's `Show all {app} notifications`, both ACCESSIBLE NAMES, which a screen
reader reads out; and the workspaces page's rule list.

WHAT COUNTS. A string of three words or more, beginning like a sentence, in one of
three positions: the value of a user-facing ATTRIBUTE, a `return` from a function,
or an assignment to a name that reads user-facing (`*error`, `*message`, `*label`,
`*hint`, `*title`, `*text`, ...).

WHAT DOES NOT COUNT, each for its own reason:

  * A DEFAULT VALUE in a `$props()` destructuring. The kit writes its fallbacks in
    English on purpose and `check-kit-defaults.py` already holds apps to
    overriding them; counting them here would teach a reader that a deliberate
    pattern is suspect, and would start this check with eight carried instances
    that are not defects.
  * A catalogue file. That is where the sentences are supposed to be.
  * A test file, and a comment line.
  * A lowercase token - `no-renderer`, `data-place` - which is an identifier
    rather than a sentence.

WHAT IT CANNOT SEE: a sentence built from pieces (`"Add " + noun`), and a literal
handed to a helper whose name this does not recognise. Both were left rather than
guessed at: a rule matching every string assignment would report the tree's paths,
class names and command tokens by the hundred.

THE CARRIED QUEUE, the same mechanism as the sibling and for the same reason. Most
of what is left belongs to lanes this one does not edit, and a gate that lands red
on somebody else's files is a gate they learn to skip. So each is a per-file COUNT:
a file that grows a new instance fails, and a file whose count drops asks to have
its number lowered. The queue can only shrink.

Run: dev/scripts/check-born-translatable.py [root]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

SOURCES = ("apps/*/src", "sdk/ui-kit/src")

#: Attributes whose value a person reads or hears. `class` and `style` are not
#: here for the obvious reason; `title` and `aria-label` are, because a tooltip
#: and an accessible name are text somebody meets.
ATTRS = (
    "aria-label", "aria-description", "title", "placeholder", "label", "hint",
    "text", "message", "summary", "caption", "addLabel", "emptyMessage",
    "errorTitle", "confirmLabel", "cancelLabel",
)
ATTR = re.compile(r'\b(?:' + "|".join(map(re.escape, ATTRS)) + r')="((?:[^"\\]|\\.){8,})"')
RETURN = re.compile(r'\breturn\s+"((?:[^"\\]|\\.){8,})"')
NAME = r"\w*(?:error|message|msg|reason|hint|note|label|title|text|placeholder|summary|caption|description|desc)\w*"
ASSIGN = re.compile(r"\b(?:" + NAME + r')\s*=\s*"((?:[^"\\]|\\.){8,})"', re.I)
#: THE SAME NAMES AS AN OBJECT FIELD, which `ASSIGN` cannot see: it wants an `=`,
#: and a catalogue of rows is written with a `:`. That is how the two brightness
#: rows in Settings carried an English sentence into a German page - `label` next
#: to them was a catalogue key, `description` was prose, and nothing looked at the
#: difference. A key is not prose (one word, no capital), so the entries that hold
#: keys stay quiet.
FIELD = re.compile(r"\b(?:" + NAME + r')\s*:\s*"((?:[^"\\]|\\.){8,})"', re.I)

#: `$props()` destructuring, whose defaults are the kit's and not this check's.
PROPS = re.compile(r"let\s*\{.*?\}\s*(?::[^=]*)?=\s*\$props\(\)", re.S)

#: A FILE MAY DECLARE ITS DATA FOREIGN. `// i18n-foreign` is already written above
#: five fixtures in this tree, each with the reason: the strings are a third
#: party's own words - another app's setting labels, a paper's title, a mail
#: subject - arriving as data. Translating them would make the fixture lie about
#: what a real bridge returns. The marker is a claim the author makes about the
#: file, and it reads as one; a file that carries it is skipped, so putting it on
#: a file that holds real copy is a thing a reviewer can see and argue with.
FOREIGN = re.compile(r"^\s*(?:/{2,}|\*|<!--)\s*i18n-foreign\b", re.M)

LETTER = re.compile(r"[A-Za-zÀ-ɏ]")

KNOWN: dict[str, tuple[int, str]] = {
    # arlen-ui's app. The second is a confirm body - "This removes the chat and
    # its messages. You cannot undo this." - which is a real one on their side.
    "apps/harness/src/routes/_difftest/+page.svelte": (1, "arlen-ui's, a dev route's demo label"),
    "apps/harness/src/lib/components/HarnessSidebar.svelte": (1, "arlen-ui's; a confirm body at :279"),
    # A deliberate dev-only pin: the viewer assigns a TypeError string to exercise
    # its own internal-error guard, so the guard can be SEEN working rather than
    # read. Suppressed on the way to the screen, never shown.
    "apps/viewers/src/routes/+page.svelte": (1, "the `internal-error` pin at :103, suppressed by the guard it tests"),
    # The kit's accessibility demo page. Its content is the demo.
    "sdk/ui-kit/src/lib/components/a11y-kitchen.svelte": (3, "the kit's demo page; the strings are the demo"),
    # THE FIELD FORM (`name: "..."`) reached these when it was added, and every one
    # of them is arlen-ui's to answer. Listed rather than silenced: a new sentence
    # in any of these files still raises the count and goes red.
    "apps/harness/src/lib/stores/conversation.ts": (5, "arlen-ui's; a fixture conversation, and theirs to mark"),
    "apps/harness/src/routes/_gatetest/+page.svelte": (2, "arlen-ui's; the gate demo's sample proposals"),
    "apps/harness/src/routes/_rendertest/+page.svelte": (2, "arlen-ui's; the render demo's chart titles"),
    "apps/store/src/lib/stores/catalog.ts": (15, "arlen-ui's app; a fixture catalogue of other people's software"),
    "sdk/ui-kit/src/routes/_i18n/+page.svelte": (1, "the kit's i18n demo route; the string is the demo"),
    # Real copy, and it is read on arlen-ui's Models page - the three tier notes
    # ("Snappy on your machine, lighter answers.") come from this store and are
    # rendered verbatim. Translating them means changing what their page calls,
    # so it lands with them rather than across the lane line.
    "apps/settings/src/lib/stores/models.ts": (7, "feeds arlen-ui's models page; the tier notes are theirs to route"),
    # A real one, and arlen-ui's: the saturation pad's accessible name.
    "sdk/ui-kit/src/lib/components/ui/color-picker/ColorPicker.svelte": (1, "arlen-ui's; the pad's aria-label at :186"),
}


def prose(value: str) -> bool:
    """Does this read as a sentence somebody wrote for a person?"""
    words = [w for w in value.split() if w]
    if len(words) < 3:
        return False
    if not LETTER.search(value):
        return False
    if not re.match(r'^[A-ZÀ-Ü"“\'{]', value):
        return False
    # An identifier, not a sentence.
    if re.match(r"^[a-z0-9-]+$", value):
        return False
    return True


def scan(path: Path) -> list[tuple[int, str]]:
    text = path.read_text(encoding="utf-8")
    if FOREIGN.search(text):
        return []
    skip = [m.span() for m in PROPS.finditer(text)]
    found: list[tuple[int, str]] = []
    offset = 0
    for number, line in enumerate(text.splitlines(True), 1):
        start, offset = offset, offset + len(line)
        stripped = line.strip()
        if stripped.startswith(("//", "*", "/*", "<!--")):
            continue
        if any(a <= start < b for a, b in skip):
            continue
        seen: set[str] = set()
        for match in (*ATTR.finditer(line), *RETURN.finditer(line), *ASSIGN.finditer(line), *FIELD.finditer(line)):
            value = match.group(1)
            if value in seen or not prose(value):
                continue
            seen.add(value)
            found.append((number, value))
    return found


def main() -> int:
    files: list[Path] = []
    for pattern in SOURCES:
        for base in ROOT.glob(pattern):
            for suffix in ("*.svelte", "*.ts"):
                files.extend(
                    p for p in base.rglob(suffix)
                    if "node_modules" not in p.parts
                    and "/i18n/" not in str(p)
                    and ".test." not in p.name
                )

    # A run that looked at nothing is not a clean run. The sibling checks learnt
    # this the hard way and so does this one.
    if not files:
        print(f"check-born-translatable: no frontend sources under {ROOT}", file=sys.stderr)
        return 2

    counts: dict[str, list[tuple[int, str]]] = {}
    for path in sorted(files):
        hits = scan(path)
        if hits:
            counts[str(path.relative_to(ROOT))] = hits

    problems: list[str] = []
    for rel, hits in sorted(counts.items()):
        allowed = KNOWN.get(rel, (0, ""))[0]
        if len(hits) > allowed:
            problems.append(
                f"  - {rel}: {len(hits)} sentence(s) written in place, {allowed} carried\n"
                + "".join(f"      :{n}  {v[:78]}\n" for n, v in hits[: allowed + 3])
            )
    for rel, (allowed, why) in sorted(KNOWN.items()):
        # Only where the file is actually in the tree being scanned. A fixture
        # root has none of them, and a carried number is a statement about this
        # repository rather than about every directory this is pointed at.
        if not (ROOT / rel).exists():
            continue
        have = len(counts.get(rel, []))
        if have < allowed:
            problems.append(
                f"  - {rel}: carried as {allowed} ({why}) and only {have} left."
                f" Lower the number so a new one cannot hide behind it.\n"
            )

    if problems:
        print("Sentences a person reads, written where the catalogue cannot see them:\n", file=sys.stderr)
        for p in problems:
            print(p, file=sys.stderr)
        print(
            "Put the sentence in the app's catalogue and read it through `$t`.\n"
            "One place chooses the language.",
            file=sys.stderr,
        )
        return 1

    carried = sum(v[0] for v in KNOWN.values())
    print(
        f"{len(files)} frontend source(s); every sentence a person reads goes through the"
        f" catalogue, {carried} carried."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
