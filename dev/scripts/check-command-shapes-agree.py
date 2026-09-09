# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that one command name means one set of arguments across the tree.

Found on 6 September: `frontend_log` is registered in TEN components and carried
THREE different argument shapes.

    files, terminal, xdg-portal, desktop-shell, settings, harness   (level, msg)
    clock, system-monitor, store                                    (level, message)
    screenshot                                                      (message)

Every app was internally consistent, so `check-invoke-shape` was green - it asks
whether a call matches the command in ITS OWN app, which is the right question
one app at a time and blind to this. What it costs is a copied line: take a
logging call from the file manager into the screenshot editor and Tauri fails to
deserialize the arguments, the command is never called, and the line you added to
diagnose something is itself the thing that goes missing. That is not
hypothetical - it happened here the same evening, to the person writing this.

Logging is the worst place for it and the likeliest, because a log line is the
thing people copy between apps without thinking, and its failure is silent by
construction.

What it looks for: every `#[tauri::command]` in `apps/` and `daemons/`, grouped
by name, comparing the argument NAMES (types are the sibling check's question).
Tauri's injected arguments - `AppHandle`, `State`, `Window` - are not part of the
wire shape and are skipped.

What it does NOT cover:

  * two NAMES for one job, which is the same trap wearing a different hat: the
    shell registers `frontend_log(level, msg)` AND `log_frontend(message)`, and
    those are two names so nothing here compares them. Recorded in the reports;
    collapsing them is nine caller edits and a decision about which name wins
  * argument TYPES, which `check-invoke-shape` compares per app
  * whether the shapes SHOULD be the same. Two commands that happen to share a
    name and do unrelated things would be a false positive; there are none today
    and the reason would go in KNOWN if there ever were
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

# An optional root, so the control can point this at a fixture tree instead of
# editing the repository. Its first cut did edit it - putting a real defect back,
# running, restoring - which is fine alone and wrong under the pre-commit hook,
# where the gates run CONCURRENTLY: another check reading the Rust sources
# mid-control sees a tree nobody wrote. It failed that way within the hour, and a
# gate that fails at random is worse than no gate.
ROOT = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

COMMAND = re.compile(
    r"#\[tauri::command[^\]]*\]\s*(?:pub\s+)?(?:async\s+)?fn\s+(\w+)\s*\(([^)]*)\)",
    re.S,
)
ARG = re.compile(r"(?:^|,)\s*(?:mut\s+)?([a-z_][a-z0-9_]*)\s*:\s*([^,]+)")
#: Tauri injects these; they never travel on the wire.
INJECTED = ("tauri::", "AppHandle", "State<", "Window", "WebviewWindow")

# Divergences that are somebody else's to settle. A queue, not an alibi.
# The COUNT is what keeps this a queue: a name-keyed exception with no number
# would hide a THIRD shape appearing under a name already carried, which is
# exactly what the control caught when this was written without one.
KNOWN: dict[str, tuple[int, str]] = {
    "frontend_log": (
        2,
        "`apps/store` is arlen-ui's live work and registers `(level, message)`; "
        "the other nine agree on `(level, msg)` since 6 September. "
        "FALSE WHEN: that app's command takes `msg`, at which point this entry "
        "goes and the check is green on its own"
    ),
}


def component(path: Path) -> str:
    rel = path.relative_to(ROOT).parts
    return rel[1] if len(rel) > 1 else rel[0]


def main() -> int:
    shapes: dict[str, dict[tuple[str, ...], set[str]]] = {}
    files = 0
    for base in (ROOT / "apps", ROOT / "daemons"):
        for path in base.rglob("*.rs"):
            if "/target/" in str(path):
                continue
            files += 1
            text = path.read_text(encoding="utf-8", errors="replace")
            for m in COMMAND.finditer(text):
                args = [
                    a.group(1)
                    for a in ARG.finditer(m.group(2))
                    if not any(s in a.group(2) for s in INJECTED)
                ]
                shapes.setdefault(m.group(1), {}).setdefault(
                    tuple(sorted(args)), set()
                ).add(component(path))

    if files == 0:
        print("check-command-shapes-agree: no Rust source found, so the scan is pointed wrong")
        return 1
    shared = sum(1 for v in shapes.values() if len(v) > 1 or sum(len(w) for w in v.values()) > 1)
    bad: list[str] = []
    carried = 0
    for name, variants in sorted(shapes.items()):
        if len(variants) < 2:
            continue
        allowed = KNOWN.get(name, (1, ""))[0]
        if len(variants) <= allowed:
            carried += 1
            continue
        lines = "\n".join(
            f"      ({', '.join(shape) or 'no arguments'}) in {', '.join(sorted(where))}"
            for shape, where in variants.items()
        )
        bad.append(f"  - `{name}` is registered with {len(variants)} argument shapes:\n{lines}")

    print(
        f"{len(shapes)} command name(s) across {files} Rust source(s);"
        f" {shared} registered in more than one component, {carried} divergence(s) carried."
        " Argument NAMES only - types are `check-invoke-shape`'s question, one app at a"
        " time, which is why it cannot see this."
    )
    if bad:
        print("\ncommand names that mean different arguments in different components:\n")
        print("\n".join(bad))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
