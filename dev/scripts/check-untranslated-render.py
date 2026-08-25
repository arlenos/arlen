#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Check that a stringified error is not drawn on a surface without a catalogue.

THE SHAPE, and why it needed a check of its own. Two gates already stand either
side of it and neither can see it. `check-refusal-language` looks for a raw error
INSIDE a `$t(...)` call - the half-translated sentence. `check-unrendered-error`
asks whether a recorded failure has a READER at all. Between them sits

    {#if $opError}<span>{$opError}</span>{/if}

where the store holds `String(e)`: it has a reader, and there is no translate call
to inspect. So the red bar in the file manager carried "destination already
exists: notes.md" in a German window, and nothing said so.

Found on 25 August by looking for the second instance after fixing the first.
There were thirteen.

WHAT COUNTS. A name that this file - or a module it IMPORTS the name from - gives
a stringified exception to, interpolated bare into markup: `{name}` or `{$name}`,
with nothing around it. The import is required rather than inferred from a shared
name, for the reason the sibling check records: matching `error` by name alone
reported three files where an unrelated local happened to share it.

WHAT DOES NOT COUNT, and each of these is a real thing the tree does:

  * `{$t(key, { reason })}` - that is the sibling check's, which words the finding
    better because it can name the key.
  * A store holding a message KEY rather than a message. `{$t($opError)}` is the
    fix this asks for and passes here, since the interpolation is a call.
  * Anything that is not fed by `String(e)`: a count, a path, a filename. The
    taint is the whole point - a surface drawing its own data is not this.

WHAT IT CANNOT SEE, and the second one was measured rather than assumed:

  * A taint reaching the markup through a component PROP or a derived store.
  * TWO HOPS - a local assigned from a tainted field (`x = outcome.reason`) and
    then drawn. That is the shape `apps/screenshot` had before 09:2x on 25 August,
    so it is not hypothetical. A pattern for it was written and measured against
    the tree: two findings, BOTH wrong (`editDraft = msg.text`, `draft = …text`,
    where `text` is a field name tainted elsewhere in the same app), and none
    right. A rule whose whole yield is false positives costs more than the hole,
    so the hole stays and is written down here instead.

THE CARRIED QUEUE. Eleven of the thirteen are in apps this lane does not edit, and
a gate that lands red on eleven files somebody else owns is a standing red - which
teaches people to pass `--no-verify`, and takes the true reds with it. So they are
carried as per-file COUNTS: a file that grows a new one fails, and a file whose
count drops asks to have its number lowered. The queue can only shrink.

Run: dev/scripts/check-untranslated-render.py [root]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path, PurePath

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

#: Where a frontend lives. The kit's shared components are held to the same rule.
SOURCES = ("apps/*/src", "sdk/ui-kit/src", "daemons/*/*/src")

#: The exception names the tree uses. Pinned, so `String(someNumber)` is not a
#: taint.
EXC = r"(?:e|err|error|ex)"

#: A name given a stringified exception: a declaration, a plain assignment, or a
#: store's `.set`, each in its template-literal form as well.
#: A FIELD of an object literal given a stringified exception:
#: `{ kind: "unavailable", reason: String(e) }`.
#:
#: Kept SEPARATE from the name taints below, and matched only as a property
#: access (`something.reason`), never as a bare `{reason}`. Field names are the
#: most ordinary words in the language - `reason`, `why`, `message` - and
#: `FmInfoPanel` has a local `{@const reason = reasonOf(read)}` holding a
#: translated sentence, three lines from a store that carries a tainted field of
#: the same name. Matching bare names reported that panel, which is correct
#: code. The access is what distinguishes the field from a local that shadows it.
FIELD_TAINT = re.compile(rf"""(?P<n>[A-Za-z_$][\w$]*)\s*:\s*(?:`[^`]*\$\{{)?String\(\s*{EXC}\s*\)""")

TAINTS = (
    re.compile(rf"""(?:const|let|var)\s+(?P<n>[A-Za-z_$][\w$]*)\s*=\s*(?:`[^`]*\$\{{)?String\(\s*{EXC}\s*\)"""),
    re.compile(rf"""(?P<n>[A-Za-z_$][\w$]*)\s*=\s*(?:`[^`]*\$\{{)?String\(\s*{EXC}\s*\)\s*;"""),
    re.compile(rf"""(?P<n>[A-Za-z_$][\w$]*)\.set\(\s*(?:`[^`]*\$\{{)?String\(\s*{EXC}\s*\)"""),
)

#: A named import, so a cross-file taint follows a link the code states.
IMPORT = re.compile(r"""import\s*\{(?P<names>[^}]*)\}\s*from\s*["'](?P<mod>[^"']+)["']""", re.S)

#: File to (how many are there today, why). The COUNT is what keeps this a queue
#: rather than a hole: a file-keyed exception hides every new instance added to an
#: already-listed file.
KNOWN: dict[str, tuple[int, str]] = {
    "apps/settings/src/routes/keyboard/+page.svelte": (
        2,
        "arlen-ui's app; `loadError` and `lastError` both drawn bare",
    ),
    "apps/settings/src/routes/keyboard/shortcuts/+page.svelte": (
        1,
        "arlen-ui's app; `lastError` drawn bare",
    ),
    "apps/settings/src/routes/ai/+page.svelte": (
        2,
        "arlen-ui's app; `statusError` and `explainError` drawn bare",
    ),
    "apps/settings/src/routes/display/+page.svelte": (
        1,
        "arlen-ui's app; `applyError` drawn bare",
    ),
    "apps/settings/src/routes/knowledge/+page.svelte": (
        1,
        "arlen-ui's app; `error` drawn bare",
    ),
    "apps/settings/src/lib/components/displays/RevertConfirmModal.svelte": (
        1,
        "arlen-ui's app; `error` drawn bare",
    ),
    "apps/harness/src/routes/agent/+page.svelte": (
        2,
        "arlen-ui's live work; `error` and `explainError` drawn bare",
    ),
    "apps/harness/src/lib/components/mint/MintFlow.svelte": (
        1,
        "arlen-ui's live work; `mintError` drawn bare",
    ),
}


def interpolations(markup: str):
    """Yield (offset, expression) for each TOP-LEVEL `{...}` in `markup`.

    Top-level is the load-bearing word. A first version matched a brace-name-brace
    pattern anywhere, and `{$t("th.failed", { reason })}` contains
    exactly that: an object shorthand is spelled the same as a bare
    interpolation. So the same defect was reported twice, once here and once by
    the sibling check that words it better - and the control caught it, which is
    the argument for writing the passing cases as well as the failing one.

    Svelte block tags (`{#if}`, `{:else}`, `{/if}`) are not expressions and are
    skipped. Nothing here parses JavaScript; it counts braces, which is enough to
    tell an outermost `{` from one inside an argument list.
    """
    i, n = 0, len(markup)
    while i < n:
        if markup[i] != "{":
            i += 1
            continue
        if i + 1 < n and markup[i + 1] in "#:/":
            i += 1
            continue
        depth, j = 1, i + 1
        while j < n and depth:
            if markup[j] == "{":
                depth += 1
            elif markup[j] == "}":
                depth -= 1
            j += 1
        if depth:
            break
        yield i, markup[i + 1 : j - 1].strip()
        i = j


def tainted_names(text: str) -> set[str]:
    return {m.group("n") for rx in TAINTS for m in rx.finditer(text)}


def main() -> int:
    files: list[Path] = []
    for pattern in SOURCES:
        for base in ROOT.glob(pattern):
            files.extend(p for p in base.rglob("*.ts") if "node_modules" not in p.parts)
            files.extend(p for p in base.rglob("*.svelte") if "node_modules" not in p.parts)

    if not files:
        print(
            "NOTHING WAS READ: no frontend sources found, so no surface was checked",
            file=sys.stderr,
        )
        return 2

    sources = {p: p.read_text(encoding="utf-8", errors="replace") for p in files}
    taints_by_stem: dict[str, set[str]] = {}
    for p, text in sources.items():
        names = tainted_names(text)
        if names:
            taints_by_stem.setdefault(p.stem, set()).update(names)

    findings: list[str] = []
    seen_per_file: dict[str, int] = {}
    for path in sorted(p for p in files if p.suffix == ".svelte"):
        text = sources[path]
        tainted = dict.fromkeys(tainted_names(text), "this file")
        for m in IMPORT.finditer(text):
            stem = PurePath(m.group("mod")).name
            for raw in m.group("names").split(","):
                local = raw.strip().split(" as ")[-1].strip()
                if local and local in taints_by_stem.get(stem, ()):
                    tainted.setdefault(local, f"`{m.group('mod')}`")
        # Fields are collected here, not inside the loop: a file whose only taint
        # is a field has an empty `tainted` map, and skipping on that alone is how
        # the first cut of this rule found nothing at all.
        fields = {m.group("n") for m in FIELD_TAINT.finditer(text)}
        if not tainted and not fields:
            continue
        # Markup only. The same `{...}` in the script block is an object literal.
        head, _, markup = text.partition("</script>")
        offset = len(head) + len("</script>") if markup else 0
        body = markup or text
        rel = str(path.relative_to(ROOT))
        for start, expr in interpolations(body):
            name = expr.lstrip("$")
            if name not in tainted:
                # A tainted FIELD only counts when it is read as one.
                head, _, field = name.rpartition(".")
                if not head or field not in fields:
                    continue
                tainted[name] = "this file"
            seen_per_file[rel] = seen_per_file.get(rel, 0) + 1
            if rel in KNOWN and seen_per_file[rel] <= KNOWN[rel][0]:
                continue
            line = text[: offset + start].count("\n") + 1
            findings.append(
                f"{rel}:{line}: `{name}` is drawn as it stands, and {tainted[name]} "
                f"sets it to a stringified error - so whatever the backend formatted "
                f"is what a person reads, in every language."
            )

    # A file whose count drops asks to have its number lowered: a count that is
    # too high reserves room for a new one to appear unreported. Scoped to this
    # tree, because KNOWN is a set of counts about ONE repo and a fixture lacks
    # those files for reasons that have nothing to do with the entries being stale.
    audits_own_list = len(sys.argv) <= 1 or ROOT == Path(__file__).resolve().parents[2]
    for rel, (declared, _) in sorted(KNOWN.items() if audits_own_list else []):
        found = seen_per_file.get(rel, 0)
        if found < declared:
            findings.append(
                f"{rel}: carried as {declared} known instance(s) and only {found} "
                f"remain. Lower the number, or drop the entry if it is zero."
            )

    if findings:
        print("a stringified error drawn with no catalogue between it and the reader:\n")
        for f in findings:
            print(f"  - {f}")
        print(
            "\nHave the backend answer with a token, hold the KEY in the store, and let "
            "the markup call the catalogue: `{$t($opError)}`."
        )
        return 1

    carried = sum(n for n, _ in KNOWN.values())
    print(
        f"{len(files)} frontend source(s); no stringified error is drawn without a "
        f"catalogue. {carried} carried in {len(KNOWN)} file(s), each bounded by its "
        f"recorded count."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
