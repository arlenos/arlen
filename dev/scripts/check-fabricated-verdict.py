# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that no handler reports a verdict about the outside world it never asked for.

The shape, found live in `AddProviderDialog.svelte` on 7 September:

    function runTest() {
      test = { kind: "testing" };
      test = { kind: "ok" };
    }

Two assignments, no call. A person typing in a new AI provider - about to paste an
API key into it - pressed Test and was told the endpoint was reachable. Nothing had
been reached. The same file's Fetch button put two invented model ids into the list
the same way.

WHY THE OTHER GATES ALL MISSED IT, which is the reason this one exists. Every check
in this tree that looks at honesty looks at a CALL:

  * `check-optimistic-write` wants a write whose failure the surface ignores
  * `check-unrendered-error` wants a recorded failure nobody reads
  * `check-fixture-on-failure` wants a catch answering with invented data
  * `check-inert-switches` wants a switch that writes nothing

Here there is no write, no failure, no catch and no switch. The handler asks nobody
and states the answer, so there is nothing for any of those to hold on to. That is
a blind spot rather than an oversight, and it survived weeks in a shipped settings
page with every gate green.

What it looks for: a function or arrow body that

  1. assigns an AFFIRMATIVE literal - "ok", "works", "connected", "success",
     "protected", "reachable", "valid", "passed" - to a name that reads as a
     verdict about something outside the process (`test`, `status`, `result`,
     `verdict`, `posture`, `reachable`, `connection`, `check`), AND
  2. contains no `invoke`, `fetch`, `await`, `navigator.`, `listen` or `emit`.

Both halves are needed. A handler that DOES call something and then sets `ok` is the
ordinary case and the other gates cover its failure paths; a local `mode = "ok"` with
no outward name is not a claim about the world.

What it does NOT cover, deliberately:

  * a fabricated verdict assembled through a helper in the same file. Following one
    hop would need call-graph work for a shape that has appeared once, and the
    version that has appeared is the direct one
  * a fabricated FAILURE. Saying something is broken when nothing was asked is also
    wrong, but it is the safe direction and it does not get a credential typed into
    it
  * Rust. A command that fabricates is a different check with different vocabulary,
    and every fabrication found so far has been in a handler
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
SKIP = ("node_modules", "/.svelte-kit/", "/build/", "/dist/", "/target/")

FN = re.compile(
    r"(?:function\s+([A-Za-z0-9_]+)\s*\([^)]*\)\s*(?::[^{]+)?\{"
    r"|(?:const|let)\s+([A-Za-z0-9_]+)\s*=\s*(?:async\s*)?\([^)]*\)\s*(?::[^=]+)?=>\s*\{)"
)
AFFIRM = re.compile(
    r"""["'](ok|works|working|connected|success|succeeded|protected|reachable|valid|passed)["']"""
)
OUTWARD = re.compile(r"\binvoke\s*\(|\bfetch\s*\(|\bawait\b|navigator\.|\blisten\s*\(|\bemit\s*\(")
VERDICT = re.compile(r"\b(test|status|result|verdict|posture|reachable|connection|check)\b", re.I)

# The longest body worth reading as one claim. A fabrication is short by nature -
# it has nothing to do - and a long function is where a call hides from a regex.
MAX_BODY = 1200

KNOWN: dict[str, str] = {}


def sources() -> list[Path]:
    out: list[Path] = []
    for pattern in ("apps/*/src/**/*.svelte", "apps/*/src/**/*.ts", "sdk/*/src/**/*.svelte"):
        out.extend(p for p in ROOT.glob(pattern) if not any(s in str(p) for s in SKIP))
    return sorted(out)


def body_from(text: str, brace: int) -> str:
    """The `{...}` block starting at `brace`, or empty when it never closes."""
    depth = 0
    for i in range(brace, len(text)):
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                return text[brace : i + 1]
    return ""


def findings_in(text: str) -> list[str]:
    out: list[str] = []
    for m in FN.finditer(text):
        name = m.group(1) or m.group(2) or "?"
        brace = text.find("{", m.start())
        if brace < 0:
            continue
        body = body_from(text, brace)
        if not body or len(body) > MAX_BODY:
            continue
        # Comments are prose about the code, not the code. A note explaining that
        # a verdict is not real must not read as the verdict.
        code = "\n".join(l for l in body.split("\n") if not l.strip().startswith("//"))
        if OUTWARD.search(code):
            continue
        for line in code.split("\n"):
            if "=" not in line or not AFFIRM.search(line):
                continue
            if VERDICT.search(line.split("=")[0]):
                out.append(f"`{name}` sets {line.strip()[:90]} and calls nothing")
                break
    return out


def main() -> int:
    files = sources()
    if not files:
        print("check-fabricated-verdict: no sources found, so the scan is pointed wrong")
        return 1
    bad: list[str] = []
    carried = 0
    for p in files:
        rel = str(p.relative_to(ROOT))
        hits = findings_in(p.read_text(encoding="utf-8", errors="replace"))
        if not hits:
            continue
        if rel in KNOWN:
            carried += len(hits)
        else:
            bad.extend(f"  - {rel}: {h}" for h in hits)
    print(
        f"{len(files)} file(s) checked for a verdict stated without asking."
        f" {carried} carried in {len(KNOWN)} known file(s)."
        " Direct assignments only: a fabrication routed through a helper is not"
        " visible here, and neither is a fabricated failure."
    )
    if bad:
        print("\nverdicts stated without asking anyone:\n")
        print("\n".join(bad))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
