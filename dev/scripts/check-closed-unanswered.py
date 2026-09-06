# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a surface does not close on an answer it threw away.

The shape, found six times in one afternoon on 6 September:

    invoke("do_the_thing", args).catch(() => {});
    close();

Two things happen and neither is visible. The action's rejection is discarded, so
nothing knows it failed; then the surface that offered the button disappears, so
there is nowhere left to say it and no control to press again. What the person
sees is the thing they clicked going away and nothing happening, which is exactly
what SUCCESS looks like from outside.

What it cost before it was named, all in the shell:

  * the launcher copied a unicode character, ignored the clipboard's answer and
    hid the window in the same tick - a refused copy left the previous clipboard
    in place and they find out at the paste
  * the same launcher killed a process that way. Refused, it closed as though it
    had worked and the only way to find out was to go looking for the process
  * the project switcher, in the top bar AND in Quick Settings, closed its panel
    and fired `set_query_and_show` into the dark; refused, one surface is gone
    and the one you asked for never came
  * the workspace overview raised a window that way on both the drag and the
    keyboard path, and its two restore siblings "reported" through `console.warn`,
    which WebKitGTK does not reliably put anywhere a session can read

WHY THIS IS A DIFFERENT CHECK FROM `check-optimistic-write`. That one looks for a
store write BEFORE a try, and its own notes explain why it declines the mirrored
shape: in a catch, a REVERT and a fake success are the same syntax and only the
direction tells them apart, which is semantic. That argument does not reach here.
A close is not ambiguous - closing is never a revert - so "discard the answer,
then take the surface away" has no honest reading, and this is separable where
the store-write variant is not.

What it looks for: a discarded rejection (`.catch(() => {})`) anywhere in the
same FUNCTION BODY as a call that closes a surface. Function-scoped rather than
by proximity, because the sibling check learned that the hard way - a 500-character
lookbehind matched writes in the function ABOVE and carried four entries that were
never defects.

What it does NOT cover:

  * a rejection discarded some other way - `catch (e) {}` around an await, or a
    promise nobody awaits at all. Both are the same defect; this matches the one
    shape the tree actually writes
  * whether the message the fix adds is the RIGHT message, or is rendered
  * a close two calls away - the helper-plus-caller pair below reaches one hop,
    not a chain
  * a close that is IMPLICIT in a component. The terminal is the live example and
    it is why this bullet is here rather than an entry in a carry list: `copyText`
    throws the clipboard's answer away and every block-context-menu copy calls it,
    but the menu closes itself inside the kit primitive, so there is no token in
    that file for any syntactic rule to find. It was found by reading and is
    recorded in the reports; nothing here will ever say it
  * `apps/harness` and `apps/store`, arlen-ui's live work

The fix that satisfies it is the one all six got: route the call through a helper
that names the failure (`shellAction` in the shell), and close only on success.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

SKIP = ("/harness/", "/store/", "node_modules", "/.svelte-kit/")

#: A rejection handed to a handler that does nothing with it.
DISCARDED = re.compile(r"\.catch\(\s*\(\s*\)\s*=>\s*\{\s*\}\s*\)")

#: Taking the surface away. `close`-prefixed names plus `dismiss`, which is what
#: this tree calls it; a bare `hide(` is deliberately NOT here, because the window
#: helpers use it for things that are not a person's surface closing under them.
CLOSER = re.compile(r"\b(?:close|close[A-Z]\w*|dismiss)\s*\(")

#: A log line that could not be written is not a claim made to anybody, so a
#: discarded rejection on one is honest. This is the only exemption: everything
#: else that fails silently was going to change something the person asked for.
LOGGING = re.compile(r"\b(?:frontend_log|log_frontend|console\.\w+)\b")

# The ones that were already there. This is a queue, not an alibi, and the COUNT
# is what keeps it one: a file-keyed exception with no number hides every new
# instance added to an already-listed file.
KNOWN: dict[str, tuple[int, str]] = {}


def blank_noncode(text: str) -> str:
    """Replace comments and string bodies with spaces, keeping every offset.

    Brace counting is what scopes a finding to its function, and a brace inside a
    comment or a template literal would move the boundary. Lengths and newlines
    are preserved so reported line numbers still point at the real line.
    """
    out = list(text)
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if c == "/" and i + 1 < n and text[i + 1] == "/":
            j = text.find("\n", i)
            j = n if j < 0 else j
            for k in range(i, j):
                out[k] = " "
            i = j
        elif c == "/" and i + 1 < n and text[i + 1] == "*":
            j = text.find("*/", i + 2)
            j = n if j < 0 else j + 2
            for k in range(i, j):
                if text[k] != "\n":
                    out[k] = " "
            i = j
        elif c in "\"'`":
            quote, j = c, i + 1
            while j < n:
                if text[j] == "\\":
                    j += 2
                    continue
                if text[j] == quote:
                    j += 1
                    break
                j += 1
            for k in range(i + 1, min(j - 1, n) + 1):
                if k < n and text[k] != "\n":
                    out[k] = " "
            i = j
        else:
            i += 1
    return "".join(out)


def enclosing_body(blank: str, pos: int) -> tuple[int, int] | None:
    """The brace range of the innermost block containing `pos`, or None."""
    depth = 0
    start = -1
    for i in range(pos - 1, -1, -1):
        c = blank[i]
        if c == "}":
            depth += 1
        elif c == "{":
            if depth == 0:
                start = i
                break
            depth -= 1
    if start < 0:
        return None
    depth = 0
    for i in range(start, len(blank)):
        c = blank[i]
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return start, i
    return None


#: A function declared in this file whose own body discards a rejection.
#:
#: The terminal is why this exists, and it is the same lesson `check-optimistic-
#: write` learned from the clock's `patch`: the swallow and the close are often in
#: two different functions. `copyText` there is three lines that throw the
#: clipboard's answer away, and every block-context-menu copy calls it and then
#: closes the menu - so a check that only looks inside one body sees an app with
#: nothing wrong in it.
DECL = re.compile(
    r"(?:function\s+(\w+)\s*\(|(?:const|let|var)\s+(\w+)\s*=\s*(?:async\s+)?(?:function\s*)?\()"
)


def swallowing_helpers(blank: str) -> set[str]:
    """Names of same-file functions that discard a rejection when called."""
    names: set[str] = set()
    for m in DECL.finditer(blank):
        name = m.group(1) or m.group(2)
        if not name:
            continue
        brace = blank.find("{", m.end())
        if brace < 0:
            continue
        span = enclosing_body(blank, brace + 1)
        if span and DISCARDED.search(blank[span[0] : span[1]]):
            names.add(name)
    return names


def findings_in(text: str) -> list[int]:
    """Line numbers of discarded rejections that sit beside a surface close."""
    blank = blank_noncode(text)
    hits: list[int] = []
    helpers = swallowing_helpers(blank)
    for name in sorted(helpers):
        for m in re.finditer(rf"\b{re.escape(name)}\s*\(", blank):
            span = enclosing_body(blank, m.start())
            if span is None:
                continue
            body = blank[span[0] : span[1]]
            if DISCARDED.search(body):
                continue  # the declaration itself, reported by the direct pass
            if CLOSER.search(body):
                hits.append(text[: m.start()].count("\n") + 1)
                break
    for m in DISCARDED.finditer(blank):
        span = enclosing_body(blank, m.start())
        if span is None:
            continue
        # The call itself, for the logging exemption. Bounded by the enclosing
        # body and keyed on the previous `;` only - an earlier cut took the
        # nearest `{` too and landed inside the call's own options object, so
        # `invoke("frontend_log", { msg })` read as a statement beginning at
        # `{ msg` and the exemption never fired. The control caught it.
        stmt_start = blank.rfind(";", span[0], m.start())
        stmt = text[(stmt_start + 1) if stmt_start > 0 else span[0] : m.start()]
        if LOGGING.search(stmt):
            continue
        body = blank[span[0] : span[1]]
        # The close must be a DIFFERENT statement, not the discarded call's own
        # name - `closePopover().catch(() => {})` would otherwise match itself.
        without = body[: m.start() - span[0]] + body[m.end() - span[0] :]
        if CLOSER.search(without):
            hits.append(text[: m.start()].count("\n") + 1)
    return sorted(set(hits))


def frontend_files() -> list[Path]:
    out: list[Path] = []
    for base in (ROOT / "apps",):
        for p in sorted(base.rglob("*.svelte")) + sorted(base.rglob("*.ts")):
            rel = "/" + str(p.relative_to(ROOT))
            if any(s in rel for s in SKIP) or p.name.endswith(".test.ts"):
                continue
            out.append(p)
    return out


def main() -> int:
    files = frontend_files()
    bad: list[str] = []
    carried = 0
    for p in files:
        rel = str(p.relative_to(ROOT))
        hits = findings_in(p.read_text(encoding="utf-8", errors="replace"))
        if not hits:
            if rel in KNOWN:
                bad.append(f"  - {rel}: carried as {KNOWN[rel][0]}, and there are none left. Drop the entry.")
            continue
        allowed = KNOWN.get(rel, (0, ""))[0]
        if len(hits) > allowed:
            where = ", ".join(f":{h}" for h in hits)
            bad.append(f"  - {rel}{where}: closes a surface after discarding the answer it depended on")
        else:
            carried += len(hits)

    print(
        f"{len(files)} frontend source(s) checked for a surface that closes on a discarded answer."
        f" {carried} carried in {len(KNOWN)} known file(s), each bounded by its recorded count."
        " A rejection discarded some other way - a bare `catch (e) {}`, or a promise nobody awaits -"
        " is the same defect and is not visible here."
    )
    if bad:
        print("\nsurfaces that close on an answer they threw away:\n")
        print("\n".join(bad))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
