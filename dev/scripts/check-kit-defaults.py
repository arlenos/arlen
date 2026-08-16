# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that an app overrides every English prose default a kit component has.

`sdk/ui-kit` components take their user-facing strings as props with English
defaults, so the component renders standalone. An app that mounts one and does
not pass a string gets the DEFAULT - English, in every locale, forever, and
worded for whatever the component was written against.

The born-translatable lint cannot see this and never will: it looks for
hardcoded strings in the app's own source, and here the string is not in the app
at all. It is in ui-kit, where it is legitimately English because that is the
source language. Nothing connects the two ends.

Both instances found by hand on 16 August were the same shape:

  * `apps/files` passed `errorTitle` but not `hintUnknown`, so a failed read
    showed the app's title above the kit's English sentence.
  * `apps/knowledge` mounted `FileBrowser` twice with neither, so every failure
    in the graph browser read "Can't open this folder / Something went wrong
    reading this folder" - untranslated, and about a folder, in an app whose
    places are the knowledge graph answering a query.

What this looks for: a `<KitComponent ...>` in `apps/*/src` that does not pass a
prop the component declares with a prose string default. Props whose default is
a short token (`"list"`, `"sm"`) are not prose and are skipped.

What it does NOT cover:

  * Whether the string the app DOES pass is a catalogue lookup rather than
    another hardcoded English literal - `check-i18n` territory, and it reaches
    the app's own source where that one can see it.
  * Components taking their text as children rather than props.
  * A prop passed through a spread. There are none today; one would read as a
    finding here and belongs in ACKNOWLEDGED with its reason.
"""

import re
import sys
from pathlib import Path

ROOT = (
    Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
)

#: A default that is prose rather than a token: two words, or capitalised.
PROSE = re.compile(r"[a-z]{3,}\s+[a-z]{3,}|^[A-Z][a-z]+")

ACKNOWLEDGED: dict[str, str] = {
    "apps/files/src/routes/_thumbtest/+page.svelte:FileBrowser": (
        "A headless render harness for thumbnail affordances, and its own header "
        "says so: 'Dev route only.' Nobody reads it in any language, and giving "
        "it a catalogue would put nine strings nobody translates into the app to "
        "keep a check quiet."
    ),
}


def prose_defaults(kit: Path) -> dict[str, dict[str, str]]:
    """`{ComponentName: {prop: default}}` for kit props with a prose default."""
    out: dict[str, dict[str, str]] = {}
    for p in kit.rglob("*.svelte"):
        text = p.read_text(encoding="utf-8", errors="replace")
        block = re.search(r"let\s*\{(.*?)\}\s*(?::|=)\s*", text, re.S)
        if not block:
            continue
        for m in re.finditer(r'^\s*(\w+)\s*=\s*"([^"]{4,})"\s*,', block.group(1), re.M):
            if PROSE.search(m.group(2)):
                out.setdefault(p.stem, {})[m.group(1)] = m.group(2)
    return out


def tags(text: str, name: str):
    """Yield (offset, attribute-text) for each `<name ...>` in `text`.

    Scans to the closing `>` while skipping anything inside `{...}`, because an
    arrow function in a prop - `onadd={() => open()}` - contains a `>` and a
    plain non-greedy match ends the tag there. That exact bug made the first
    version of this scan report two settings pages that pass the prop three
    lines below where it stopped looking.
    """
    for m in re.finditer(rf"<{name}\b", text):
        i = m.end()
        depth = 0
        while i < len(text):
            c = text[i]
            if c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
            elif c == ">" and depth == 0:
                break
            i += 1
        yield m.start(), text[m.end():i]


def main() -> int:
    kit = ROOT / "sdk/ui-kit/src/lib/components"
    if not kit.is_dir():
        print(f"NOTHING WAS READ: no kit components at {kit}", file=sys.stderr)
        return 2
    defaults = prose_defaults(kit)
    if not defaults:
        print("NOTHING WAS READ: no kit component declares a prose default", file=sys.stderr)
        return 2

    findings: list[str] = []
    mounts = 0
    for p in (ROOT / "apps").glob("*/src/**/*.svelte"):
        if "node_modules" in p.parts:
            continue
        text = p.read_text(encoding="utf-8", errors="replace")
        for comp, props in defaults.items():
            for off, attrs in tags(text, comp):
                mounts += 1
                missing = [k for k in sorted(props) if not re.search(rf"\b{k}\s*=", attrs)]
                rel = p.relative_to(ROOT)
                key = f"{rel}:{comp}"
                if not missing or key in ACKNOWLEDGED:
                    continue
                line = text[:off].count("\n") + 1
                findings.append(
                    f"{rel}:{line}: <{comp}> does not pass {', '.join(missing)}, so the "
                    f"kit's English default renders in every locale"
                )

    print(
        f"{mounts} kit-component mount(s) checked against "
        f"{sum(len(v) for v in defaults.values())} prose default(s) in {len(defaults)} "
        f"component(s). Only whether a string is PASSED: whether the value is a "
        f"catalogue lookup is the i18n lint's, and it can see that one."
    )
    if findings:
        print("\nEnglish that reaches every locale:\n")
        for f in findings:
            print(f"  - {f}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
