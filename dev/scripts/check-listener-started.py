#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""A function that starts a listener has to be started by something.

WHY THIS EXISTS. On 4 September two feeds were written and called by nothing.
`watchJobs` subscribed the Activity zone to the daemon's job broadcasts and no
layout ever ran it; `watchForPrints` would have been the same an hour later. Both
compiled, both typechecked, and both read as finished features - a store exporting
a function that sets up a subscription looks identical whether or not anybody
subscribes.

The failure is quiet in the worst way. A feed nobody starts is indistinguishable
from a feed nobody publishes on: the surface renders its empty state, which is
usually correct-looking, and the thing that would have filled it never runs. That
is the same shape as the missing MIME name that made the Windows prompt inert the
same morning, and as the portal interface nothing routed to - a mechanism that is
complete and unreachable.

WHAT COUNTS AS STARTED. Any call to the function from a `.ts` or `.svelte` file
other than its own declaration. Import aliases are resolved: a component that
imports `{ installListener as installModuleListener }` and calls the alias is
calling the function, and a scanner that missed that would report a live feed as
dead - which the first cut of this did.

Run: dev/scripts/check-listener-started.py [tree]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

# Where a listener-starting export can live. Frontends only: this is about a
# webview subscribing to its host.
SOURCE_ROOTS = ["apps", "daemons/xdg-portal/picker-ui/src"]

# An exported function whose body subscribes to something.
EXPORTED = re.compile(r"export\s+(?:async\s+)?function\s+(\w+)\s*\(")
# The subscribe calls that make a function a feed rather than a helper.
SUBSCRIBES = re.compile(r"\blisten\s*<[^>]*>\s*\(|\blisten\s*\(|addEventListener\s*\(")

# Exports that start a listener nothing calls, with why and what would end it.
# MAY SHRINK, MAY NOT GROW without a reason: an entry is a claim that a feed is
# deliberately dark, which is a thing to decide rather than to inherit.
NOT_STARTED: dict[str, str] = {
    "installListener": (
        "the Tier 2 module-worker pool. Turned off at the call site with its own "
        "note: installing it at Waypointer mount made the card stretch to fill a "
        "layer-shell window anchored to all four edges. FALSE WHEN the worker pool "
        "is wired from a dedicated route"
    ),
}


def sources() -> list[Path]:
    out: list[Path] = []
    for root in SOURCE_ROOTS:
        base = ROOT / root
        if not base.is_dir():
            continue
        for pattern in ("*.ts", "*.svelte"):
            out += [
                p
                for p in base.rglob(pattern)
                if "node_modules" not in p.parts and "build" not in p.parts
            ]
    return sorted(out)


def body_of(text: str, open_brace: int) -> str:
    """The function body, bounded by its own closing brace.

    A fixed window instead reads into whatever follows, which is how the first
    cut called `openPrintDialog` a listener: the function two below it subscribes
    and the window reached that far. A scanner whose answer depends on how much
    text it happened to look at is measuring the file's layout.
    """
    depth = 0
    for i in range(open_brace, len(text)):
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                return text[open_brace : i + 1]
    return text[open_brace:]


def is_comment(line: str) -> bool:
    """Whether a line is prose rather than code.

    A comment naming a function is not a call, and counting one as a call is how
    the first cut reported a deliberately-dark feed as live - the note explaining
    why it stays off mentions it by name.
    """
    t = line.strip()
    return t.startswith("//") or t.startswith("/*") or t.startswith("*") or t.startswith("///")


def aliases(text: str) -> dict[str, str]:
    """Local name to imported name, for `import { a as b }`."""
    found: dict[str, str] = {}
    for block in re.findall(r"import\s*\{([^}]*)\}\s*from", text, re.S):
        for part in block.split(","):
            m = re.match(r"\s*(\w+)\s+as\s+(\w+)\s*$", part)
            if m:
                found[m.group(2)] = m.group(1)
    return found


def main() -> int:
    files = sources()
    if not files:
        print("no frontend sources found; the layout moved and this check did not")
        return 1

    texts = {p: p.read_text(encoding="utf-8", errors="replace") for p in files}

    # Every exported function that subscribes, and where it is declared.
    feeds: dict[str, Path] = {}
    for path, text in texts.items():
        if path.suffix != ".ts":
            continue
        for m in EXPORTED.finditer(text):
            name = m.group(1)
            brace = text.find("{", m.end())
            if brace == -1:
                continue
            if SUBSCRIBES.search(body_of(text, brace)):
                feeds[name] = path

    started: set[str] = set()
    for path, text in texts.items():
        local = aliases(text)
        for name in feeds:
            # A call by its own name, or by a name this file imported it as.
            called_as = [name] + [a for a, real in local.items() if real == name]
            for alias in called_as:
                for line in text.split("\n"):
                    stripped = line.strip()
                    if is_comment(line) or stripped.startswith("export "):
                        continue
                    # A Svelte action is never called by anything in the file: the
                    # framework calls it when the element mounts, so `use:name` IS
                    # the start. Missing this reported two live actions as dark.
                    if re.search(rf"\buse:{re.escape(alias)}\b", line):
                        started.add(name)
                        break
                    if not re.search(rf"\b{re.escape(alias)}\s*\(", line):
                        continue
                    # `void name;` is a reference that silences a linter about an
                    # unused import. `void name();` is a CALL - fire-and-forget of
                    # a promise, which is how two of these feeds are started. The
                    # first cut treated both as references and called them dark.
                    if re.match(rf"^void\s+{re.escape(alias)}\s*;", stripped):
                        continue
                    started.add(name)
                    break

    dark = sorted(set(feeds) - started)
    problems: list[str] = []
    for name in dark:
        if name in NOT_STARTED:
            continue
        problems.append(
            f"{feeds[name].relative_to(ROOT)}: `{name}` starts a listener and "
            f"nothing calls it, so the feed never runs. A surface fed by it shows "
            f"its empty state, which is indistinguishable from having nothing to "
            f"show. Call it where the surface is mounted, or name it in "
            f"NOT_STARTED with why and what would end that."
        )
    # Only in a tree that actually holds these. A fixture holds none of them, and
    # reporting every entry as stale there buries whatever the fixture was
    # testing - the trap `check-fixture-deletes.py` names in its own words, hit
    # here on this check's first run against its own control.
    present = {n for n in NOT_STARTED if n in feeds or n in started}
    stale = sorted(set(NOT_STARTED) - set(dark)) if present else []
    for name in stale:
        if name in feeds:
            problems.append(
                f"`{name}` is listed as not started and something calls it now. "
                f"Drop the entry: a list of dark feeds that includes live ones is "
                f"one nobody checks."
            )
        else:
            # The hole this gate had on its first run: three entries named
            # functions the fixed detector no longer sees as feeds at all, and a
            # list keyed on names alone would have carried them for ever.
            problems.append(
                f"`{name}` is listed as a dark feed and is not a listener-starting "
                f"export any more. Drop the entry: an exemption for something that "
                f"no longer exists is one nobody can check."
            )

    if problems:
        print("listeners that never start:\n")
        for p in problems:
            print(f"  - {p}")
        return 1

    print(
        f"{len(feeds)} listener-starting export(s); {len(feeds) - len(dark)} started, "
        f"{len(dark)} dark on purpose"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
