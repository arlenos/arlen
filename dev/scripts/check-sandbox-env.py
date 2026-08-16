#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A sandbox worker must be spawned with an empty environment.

The workers under `ai/ai-sandbox` exist to handle input somebody else chose: a
document, an image, an audio file. They are fenced with Landlock and a seccomp
allowlist, and that allowlist deliberately permits stdout, because stdout is how
a worker returns what it extracted.

That is the hole this guards. A worker that inherits the calling daemon's
environment holds every API key, token and socket path that daemon happens to
carry, and it does not need a network syscall to leak one - it can write the
value into its own output, which arrives back as "extracted document text" and
goes on to the model. On 16 August `run_worker` was spawning all five workers
that way; nothing was leaking, and nothing was stopping it either.

So: any `Command::new` that spawns something with `sandbox` in the name must
clear the environment. Production code only - a test that drives a worker binary
directly is not shipping, and holds no secrets worth the noise.

Run: python3 dev/scripts/check-sandbox-env.py
"""

import os
import re
import sys

# An explicit root so the control can point this at a tree built to fail. Without
# one a check can only ever be watched passing, which is the state it shares with
# a check that cannot fail.
ROOT = (
    os.path.abspath(sys.argv[1])
    if len(sys.argv) > 1
    else os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
)
SKIP_DIRS = {"target", "node_modules", "mkosi.builddir", ".git"}

SPAWN = re.compile(r"Command::new\s*\(")
# The chain ends at the first `;` at or after the spawn. Rust builder chains for a
# process are single statements, so this is the whole thing including any
# `.stdin(...)`, `.env_clear()` and `.spawn()`.
CFG_TEST = re.compile(r"#\[cfg\(test\)\]")


def chains(text: str):
    """Every `Command::new(...)` builder chain in `text`, as (line, chain)."""
    for m in SPAWN.finditer(text):
        end = text.find(";", m.start())
        if end == -1:
            end = len(text)
        yield text.count("\n", 0, m.start()) + 1, text[m.start():end]


def test_module_spans(text: str) -> list:
    """(start, end) offsets of `#[cfg(test)]` modules, so they can be skipped."""
    spans = []
    for m in CFG_TEST.finditer(text):
        brace = text.find("{", m.end())
        if brace == -1:
            continue
        depth, i = 0, brace
        while i < len(text):
            if text[i] == "{":
                depth += 1
            elif text[i] == "}":
                depth -= 1
                if depth == 0:
                    break
            i += 1
        spans.append((m.start(), i))
    return spans


def main() -> int:
    problems = []
    for base, dirs, files in os.walk(ROOT):
        dirs[:] = [d for d in dirs if d not in SKIP_DIRS]
        # A test that runs a worker binary is not the shipped path.
        if os.path.basename(base) == "tests":
            continue
        for name in files:
            if not name.endswith(".rs"):
                continue
            path = os.path.join(base, name)
            try:
                with open(path, encoding="utf-8", errors="replace") as fh:
                    text = fh.read()
            except OSError:
                continue
            if "Command::new" not in text:
                continue
            spans = test_module_spans(text)
            for line, chain in chains(text):
                off = text.find(chain)
                if any(s <= off <= e for s, e in spans):
                    continue
                # Only chains that spawn a sandbox worker. The name is the signal:
                # every worker binary is `arlen-<what>-sandbox`, and the variable
                # holding its path is named for it too.
                if "sandbox" not in chain.lower():
                    continue
                if ".env_clear()" in chain:
                    continue
                rel = os.path.relpath(path, ROOT)
                problems.append(f"{rel}:{line}: spawns a sandbox worker without .env_clear()")

    if problems:
        print("sandbox workers must be spawned with an empty environment:")
        for p in problems:
            print(f"  {p}")
        print()
        print("A worker parses input somebody else chose and is allowed to write")
        print("stdout. Inheriting our environment lets it return a secret as text.")
        return 1
    print("check-sandbox-env: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
