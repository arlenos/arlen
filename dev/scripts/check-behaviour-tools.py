#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Check that a behaviour only declares privileged tools the proxy actually registers.

WHY. A behaviour's `tools:` list becomes the model's capability context - the prompt
telling it what it may reach for. Nothing checks the names against the plugin that
registers them, and on 19 August every behaviour in the tree declared `graph.query`
while the tool is called `graph.read`, and `tidy-downloads` declared `fs.list` and
`fs.move`, which nothing registers at all. The model is told it holds tools it cannot
call and nothing about the one it can.

It is not a hole - the capability context is prompt context and the gate is the real
authority - which is exactly why nothing caught it: everything still works, the runs
still gate, and the only symptom is a model reaching for a name that is not there.

SCOPE: PRIVILEGED-PROXY TOOLS ONLY (`graph.`, `fs.`, `os.` - the same prefixes
`is_privileged_proxy_tool` uses). Those are ours: the daemon proxies them and the
plugin declares them, so a name with no registration is a mistake we made. A generic
tool is the engine's own (pi ships its own set and can grow it), so this says nothing
about those rather than guessing.

BASELINE. The three mismatches above are recorded in KNOWN_MISMATCH rather than
fixed here: which name is canonical is a decision - `graph.query` reads better and
`graph.read` is what ships - and renaming across six manifests, the gate and the
registry on a hunch is how a working tree stops working. New ones fail.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BEHAVIOURS = ROOT / "ai/ai-skills/behaviours"
PROXY = ROOT / "ai/pi-plugins/src/proxy.ts"

#: The prefixes that mark a tool as one WE proxy, mirroring
#: `is_privileged_proxy_tool` in `daemons/ai-engine-daemon/src/pi_run.rs`.
PRIVILEGED = ("graph.", "fs.", "os.")

#: Declared names that do not resolve, with the reason they are carried. Each
#: entry is a behaviour/tool pair, so fixing one does not silently excuse another.
KNOWN_MISMATCH: dict[str, str] = {
    "graph.query": (
        "every behaviour declares it and the registered tool is `graph.read`; "
        "which name is canonical is a planner decision, not a rename to do on a hunch"
    ),
    "fs.list": "tidy-downloads declares it; no OS mutation proxy is registered yet",
    "fs.move": "tidy-downloads declares it; no OS mutation proxy is registered yet",
}


def registered_tools() -> set[str]:
    """The tool names the pi proxy plugin declares."""
    text = PROXY.read_text(encoding="utf-8")
    return set(re.findall(r'name:\s*"([^"]+)"', text))


def declared_tools() -> list[tuple[str, str]]:
    """`(behaviour, tool)` for every tool a behaviour manifest declares."""
    out: list[tuple[str, str]] = []
    for skill in sorted(BEHAVIOURS.glob("*/SKILL.md")):
        in_tools = False
        for line in skill.read_text(encoding="utf-8").splitlines():
            if line.startswith("tools:"):
                in_tools = True
                continue
            if in_tools:
                # The block ends at the next top-level key.
                if line and not line.startswith(" "):
                    break
                m = re.match(r"\s+([A-Za-z0-9_.]+):", line)
                if m:
                    out.append((skill.parent.name, m.group(1)))
    return out


def main() -> int:
    if not BEHAVIOURS.is_dir() or not PROXY.is_file():
        print("behaviours or the proxy plugin moved; this check did not", file=sys.stderr)
        return 2

    registered = registered_tools()
    if not registered:
        print(f"NOTHING WAS READ: no tool names in {PROXY}", file=sys.stderr)
        return 2

    declared = declared_tools()
    if not declared:
        print(f"NOTHING WAS READ: no behaviour declares a tool under {BEHAVIOURS}", file=sys.stderr)
        return 2

    problems: list[str] = []
    carried: list[str] = []
    checked = 0
    for behaviour, tool in declared:
        if not tool.startswith(PRIVILEGED):
            continue
        checked += 1
        if tool in registered:
            continue
        if tool in KNOWN_MISMATCH:
            carried.append(f"{behaviour} declares {tool} - {KNOWN_MISMATCH[tool]}")
            continue
        problems.append(
            f"{behaviour} declares `{tool}` and the proxy plugin registers no such tool.\n"
            f"    The model is told it holds this and cannot call it. Register it in "
            f"{PROXY.relative_to(ROOT)}, use the name that is registered, or add it to "
            f"KNOWN_MISMATCH with the reason it waits."
        )

    if carried:
        print("carried, with a reason (see KNOWN_MISMATCH):")
        for c in sorted(carried):
            print(f"  {c}")
        print()

    if problems:
        print("a behaviour reaching for a tool nothing registers:\n")
        for p in problems:
            print(f"  - {p}")
        return 1

    print(
        f"{checked} privileged tool declaration(s) across "
        f"{len({b for b, _ in declared})} behaviour(s); every one is registered or carried."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
