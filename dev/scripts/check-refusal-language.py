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

LAUNDERED THROUGH A VARIABLE. This originally caught only the inline form and said
so, on the grounds that tracking assignments guesses at dataflow. That caution was
right about the method and wrong about the cost: within hours the same defect
turned up twice more in exactly the laundered shape - the task manager filling
`sm.err.stop` from a `reason` set to `String(e)` seven lines earlier, and the
shell's module host filling `sh.module.didNotMount` from `mountError`. A gate that
matches only the shape its author last happened to write is the recurring lesson
of this directory.

So it is widened, but NARROWLY, without pretending to do dataflow: a name assigned
a stringified exception ANYWHERE in the file, appearing as a value in a translate
call's argument object ANYWHERE in the same file. No control flow is reasoned
about, nothing crosses a file boundary, and the two facts either both appear or
they do not. A name reassigned to a token before use stops matching, because it is
then no longer assigned `String(e)` - which is what the fix looks like.

ACROSS A FILE. A store setting `String(e)` and a page interpolating it is the same
defect again, and it was left out of the first widening as out of reach. It is
not, as long as the link is an IMPORT rather than a guess: the consuming file must
name the symbol in an `import { ... } from "..."` whose module is the file that
taints it. Matching on the name alone was tried first and reported three sites
where a local `error` in a catch happened to share a name with an unrelated one in
another file - which is what "confident nonsense" looks like, and why the import is
required rather than assumed.

WHAT IT STILL CANNOT SEE. A raw error handed straight into a component prop, or
assigned into state and rendered with no catalog call at all -
`check-unrendered-error` is the gate for that side. A taint reaching a page
through two hops, or through a default import, is also outside it.

Run: dev/scripts/check-refusal-language.py [root]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path, PurePath

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

#: Where a frontend lives. The SDK's shared components are held to the same rule.
SOURCES = ("apps/*/src", "sdk/ui-kit/src")

#: `$t("key", { ... String(e) ... })` and `get(t)("key", { ... })`, on one line.
#: The catalog key is required, so this cannot fire on an unrelated call taking an
#: object; the stringified exception is required, so it cannot fire on real data.
CALL = re.compile(
    r"""\$?t\)?\(\s*["'](?P<key>[a-zA-Z][\w.]*)["']\s*,\s*\{[^}]*String\(\s*(?:e|err|error|ex)\s*\)""",
)

#: The same call, but capturing the argument object so a laundered name can be
#: looked for inside it. Kept separate from CALL so the inline finding keeps its
#: own wording, which names the defect more precisely.
CALL_ARGS = re.compile(
    r"""\$?t\)?\(\s*["'](?P<key>[a-zA-Z][\w.]*)["']\s*,\s*\{(?P<args>[^}]*)\}""",
)

#: A named import, so a cross-file taint is followed along a link the code states
#: rather than one inferred from a shared name. `import { a, b as c } from "./x"`.
IMPORT = re.compile(r"""import\s*\{(?P<names>[^}]*)\}\s*from\s*["'](?P<mod>[^"']+)["']""", re.S)

#: A name given a stringified exception: `const msg = String(e)`, a plain
#: `saveError = String(e)`, a store's `.set(String(e))`, and the template-literal
#: forms of each. The exception name is pinned to the four the tree uses, so this
#: cannot fire on `String(someNumber)`.
EXC = r"(?:e|err|error|ex)"
TAINTS = (
    # A FIELD of an object literal, which is how a tagged refusal is built:
    # `failure = { problem: "launch", reason: String(e) }`, then
    # `$t(key, { reason: failure.reason })`. Two apps carried that shape past this
    # check for a day - mail and calendar, both found by rendering rather than by
    # the rule - because the taint was never bound to a NAME.
    re.compile(rf"""(?P<n>[A-Za-z_$][\w$]*)\s*:\s*(?:`[^`]*\$\{{)?String\(\s*{EXC}\s*\)"""),
    re.compile(rf"""(?:const|let|var)\s+(?P<n>[A-Za-z_$][\w$]*)\s*=\s*(?:`[^`]*\$\{{)?String\(\s*{EXC}\s*\)"""),
    re.compile(rf"""(?P<n>[A-Za-z_$][\w$]*)\s*=\s*(?:`[^`]*\$\{{)?String\(\s*{EXC}\s*\)\s*;"""),
    re.compile(rf"""(?P<n>[A-Za-z_$][\w$]*)\.set\(\s*(?:`[^`]*\$\{{)?String\(\s*{EXC}\s*\)"""),
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
    # NOT argued for - recorded. `apps/store` is arlen-ui's live work and the
    # coder does not edit it, so the finding is held here where its owner can see
    # it rather than left as a standing red that teaches everybody to pass
    # `--no-verify`. `updateFailure` is set from `String(e)` in
    # `apps/store/src/lib/stores/updates.ts` and finishes `st.upd.failed`; the
    # fix is the usual one, a token from the backend and the sentence here.
    "apps/store/src/routes/updates/+page.svelte:54": (
        "arlen-ui's app; reported to its owner rather than edited from this lane"
    ),
    "apps/viewers/src/routes/+page.svelte:539": (
        "guarded by `readsAsInternal`: an internal-looking error falls to "
        "`v.couldNotOpenUnknown`. The third copy of that predicate, which is the "
        "argument for its home being the kit"
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

    # Read once. Every file is needed twice - for its own lines, and as a possible
    # source of a name another file imports - and re-reading 954 files for the
    # second pass costs more than holding them.
    sources = {p: p.read_text(encoding="utf-8", errors="replace") for p in files}

    def tainted_names(text: str) -> set[str]:
        return {m.group("n") for rx in TAINTS for m in rx.finditer(text)}

    #: Module stem to the names that file gives a stringified exception to. Keyed
    #: by stem because an import path is written `$lib/stores/meeting` or
    #: `./meeting` and resolving either to a real path is more machinery than the
    #: question needs; a same-named module elsewhere would at worst make this
    #: report a site somebody still has to look at.
    taints_by_stem: dict[str, set[str]] = {}
    for p, text in sources.items():
        names = tainted_names(text)
        if names:
            taints_by_stem.setdefault(p.stem, set()).update(names)

    problems: list[str] = []
    seen_acknowledged: set[str] = set()
    for path in sorted(files):
        text = sources[path]
        # Names this file gives a stringified exception to. Collected over the
        # whole file before the line walk, because the assignment is usually in a
        # catch and the interpolation is in the markup far below it.
        tainted = {n: "this file" for n in tainted_names(text)}
        # Names this file imports whose source module taints them. `b as c` binds
        # `c`, so the local name is what the argument object would carry.
        for m in IMPORT.finditer(text):
            stem = PurePath(m.group("mod")).name
            for raw in m.group("names").split(","):
                local = raw.strip().split(" as ")[-1].strip()
                if local and local in taints_by_stem.get(stem, ()):
                    tainted.setdefault(local, f"`{m.group('mod')}`")
        for n, line in enumerate(text.splitlines(), 1):
            where = f"{path.relative_to(ROOT)}:{n}"
            match = CALL.search(line)
            if match:
                if where in ACKNOWLEDGED:
                    seen_acknowledged.add(where)
                    continue
                problems.append(
                    f"{where}: `{match.group('key')}` is completed with a "
                    f"stringified error, so half the sentence is in the reader's language and "
                    f"half is whatever the backend formatted"
                )
                continue
            if not tainted:
                continue
            laundered = CALL_ARGS.search(line)
            if not laundered:
                continue
            args = laundered.group("args")
            for name in sorted(tainted):
                # Word-bounded and preceded by a separator, so `reason` does not
                # match inside `reasonKey`, and a `$`-prefixed store read counts.
                if not re.search(rf"[:\s({{,.]\$?[\w.]*{re.escape(name)}\b", args):
                    continue
                if where in ACKNOWLEDGED:
                    seen_acknowledged.add(where)
                    break
                problems.append(
                    f"{where}: `{laundered.group('key')}` is completed with `{name}`, "
                    f"which {tainted[name]} sets to a stringified error - the same "
                    f"half-translated sentence, reached through a variable"
                )
                break

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
