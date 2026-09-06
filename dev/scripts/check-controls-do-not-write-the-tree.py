# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a control does not write into the repository it is checking.

Found on 7 September, in two checks written the same day:

    const path = join(root, "apps", "screenshot", "src-tauri", "src", "lib.rs");
    const original = readFileSync(path, "utf8");
    writeFileSync(path, original.replace(FIXED, BROKEN));   // put the defect back
    ...run the check, require a refusal...
    writeFileSync(path, original);                          // and restore it

Read alone it is careful: it proves the check goes red against the REAL defect
rather than a fixture's imitation of it, and it puts the file back in a `finally`.
Run the way controls are actually run it is not. `.githooks/pre-commit` runs the
gates CONCURRENTLY - its own comment says so, because that is what keeps it near
fifteen seconds - so while one control has the tree broken, every other check is
reading it. What comes out is a failure in a check that has nothing to do with
the change being committed, and it is not reproducible when you run that check
by hand afterwards.

That is what happened: `check-command-shapes-agree` went red on a commit touching
the text editor, passed on its own a second later, and the cause was its own
sibling control editing `apps/screenshot` at the same moment. A gate that fails
at random is worse than no gate, because the first thing it teaches is to re-run
until it passes.

The fix both took: the check reads its tree from an optional argument, and the
control builds a small one under `lib/fixture.mjs` and points the check at it.

What it looks for: a write - `writeFileSync`, `rmSync`, `renameSync`,
`appendFileSync`, `unlinkSync` - whose path is the repository root or one hop
from it. One hop matters and is not a detail: the first version of this looked
only at the write's own argument, and both real cases passed the path through a
`const` first, so it found nothing and would have shipped saying so.

What it does NOT cover:

  * two hops, or a path built by a helper
  * anything inside a template literal, which is deliberate: a
    control's fixture is code held as text, and reading it as code made this
    check's first run flag its own control
  * a control that shells out to something that writes
  * reading the tree, which is the whole point of most of these
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
SCRIPTS = ROOT / "dev" / "scripts"

WRITE = re.compile(r"\b(writeFileSync|rmSync|renameSync|appendFileSync|unlinkSync)\s*\(\s*([^,)]+)")
#: A name bound to the repository root, as every control spells it.
ROOT_BIND = re.compile(r'(?:const|let)\s+(\w+)\s*=\s*join\(\s*here\s*,\s*"\.\."\s*,\s*"\.\."')
JOIN_BIND = re.compile(r"(?:const|let)\s+(\w+)\s*=\s*join\(([^;]*)\)")

KNOWN: dict[str, tuple[int, str]] = {}


def blank_strings(text: str) -> str:
    """Replace TEMPLATE literal bodies with spaces, keeping every offset.

    Backticks only, and the narrowing is the point. Blanking ordinary strings too
    took `join(here, "..", "..")` apart, which is the only way a control names the
    repository root - so every file looked rootless and the check went quiet.
    Blanking neither made it read a control's fixture as the control's own code
    and flag this very file. Fixture bodies here are template literals and real
    paths are quoted strings, so the line between them is exactly the backtick.

    A control's FIXTURE is JavaScript held in a template literal, so without this
    the check reads the code a control writes into a temp tree as if the control
    ran it - and the very first thing it flagged was this file, whose fixture is a
    faithful copy of the defect it exists to catch.
    """
    out = list(text)
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if c == "`":
            quote, j = c, i + 1
            while j < n:
                if text[j] == "\\":
                    j += 2
                    continue
                if text[j] == quote:
                    break
                j += 1
            for k in range(i + 1, min(j, n)):
                if text[k] != "\n":
                    out[k] = " "
            i = j + 1
        elif c == "/" and i + 1 < n and text[i + 1] == "/":
            j = text.find("\n", i)
            i = n if j < 0 else j
        else:
            i += 1
    return "".join(out)


def findings_in(raw: str) -> list[tuple[int, str, str]]:
    # Everything reads the blanked text, and it can because only backticks are
    # blanked: `join(here, "..", "..")` in real code survives, the same line
    # inside a fixture template does not. An earlier cut read the bindings from
    # the RAW text to keep that pattern intact, and then found the root binding
    # inside this file's own fixture and reported this file.
    text = blank_strings(raw)
    roots = {m.group(1) for m in ROOT_BIND.finditer(text)}
    if not roots:
        return []
    derived = {
        m.group(1)
        for m in JOIN_BIND.finditer(text)
        if any(re.search(rf"\b{re.escape(r)}\b", m.group(2)) for r in roots)
    }
    names = roots | derived
    out = []
    for m in WRITE.finditer(text):
        if any(re.search(rf"\b{re.escape(n)}\b", m.group(2)) for n in names):
            out.append((text[: m.start()].count("\n") + 1, m.group(1), m.group(2).strip()))
    return out


def main() -> int:
    controls = sorted(SCRIPTS.glob("test-*.mjs")) if SCRIPTS.is_dir() else []
    bad: list[str] = []
    carried = 0
    for p in controls:
        rel = str(p.relative_to(ROOT))
        hits = findings_in(p.read_text(encoding="utf-8", errors="replace"))
        allowed = KNOWN.get(rel, (0, ""))[0]
        if len(hits) > allowed:
            for line, call, arg in hits:
                bad.append(f"  - {rel}:{line}: {call}({arg}) writes inside the repository")
        else:
            carried += len(hits)
    print(
        f"{len(controls)} control(s) checked for a write into the tree they check."
        f" {carried} carried in {len(KNOWN)} known file(s)."
        " One hop from the root, so a path built by a helper or through two variables"
        " is not visible here."
    )
    if bad:
        print("\ncontrols that edit the repository while other checks are reading it:\n")
        print("\n".join(bad))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
