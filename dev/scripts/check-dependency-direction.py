#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that crates which state they do not depend on a layer still do not.

`daemons/knowledge/src/typed_read.rs` explains why its typed-value encoder and
identifier validator are written out by hand rather than imported:

    reimplemented daemon-side so the graph daemon stays dependency-free
    (it must not pull the AI layer in)

That is a rule with no enforcement and an obvious-looking edit that breaks it. The
duplication is the visible cost of the rule and the dependency is the invisible
one, so a future reader sees only the cost: "dedupe the typed-value encoder
against `arlen-ai-core`" is a reasonable commit message that quietly makes the
graph daemon require the AI layer to build. The knowledge graph is the store the
whole system reads; whether it can be built and run without the assistant is an
architectural property, not a preference, and it is the same property the undo
service was split out to keep.

**Direct dependencies only, deliberately.** This reads each crate's own
`Cargo.toml`, which is the thing the crate's authors control and the thing an edit
like the one above touches. A transitive pull through some other crate would also
breach the spirit, and catching that needs the resolved graph rather than the
manifest; if that ever matters, `cargo metadata` is the tool and this is the wrong
file. Saying which of the two this checks is the point.

Run: dev/scripts/check-dependency-direction.py [tree]
"""

import re
import sys
from pathlib import Path

#: crate path -> (forbidden dependency-name prefixes, why).
#:
#: Each entry exists because the crate SAYS it holds the rule. Adding one here
#: without a statement in the crate would be this file inventing architecture.
RULES = {
    "daemons/knowledge": (
        ("arlen-ai", "ai-core", "ai-skills", "ai-engine"),
        "the graph daemon must build and run without the AI layer "
        "(daemons/knowledge/src/typed_read.rs)",
    ),
}

# The undo service was the obvious second entry and it is deliberately NOT here.
# Its rule is about runtime - "nothing in this binary may learn to ask whether it
# is running" - and it holds it; `daemons/undo-service/tests/` checks that. It
# does depend on `arlen-ai-undo-core` and `arlen-ai-undo-proto`, which are the
# undo log format and the signer protocol, shared with the agent that also writes
# entries. Linking a record format is not consulting a switch. Adding it here
# would have made this file assert an architecture the crate never claimed, which
# is what the note above the table is about; the first draft did exactly that and
# the gate reported the tree broken on its own invention.

#: A dependency line: `name = ...` or `name.workspace = true`, inside a deps table.
DEP_LINE = re.compile(r"^\s*([A-Za-z0-9_-]+)\s*(?:=|\.)")

#: Any `[dependencies]`, `[dev-dependencies]`, `[build-dependencies]`, and their
#: `[target.'cfg(...)'.dependencies]` forms.
DEPS_TABLE = re.compile(r"^\s*\[(?:[^\]]*\.)?(?:dev-|build-)?dependencies(?:\.[^\]]+)?\]")
OTHER_TABLE = re.compile(r"^\s*\[")


def declared_dependencies(manifest: str) -> list[str]:
    """Every dependency name a manifest declares, across all dependency tables.

    A `[dependencies.foo]` sub-table names `foo` in its header rather than on a
    line, so the header is read too - otherwise the long form of a dependency
    would be invisible to this check while being the same dependency.
    """
    names: list[str] = []
    in_deps = False
    for line in manifest.splitlines():
        if OTHER_TABLE.match(line):
            in_deps = bool(DEPS_TABLE.match(line))
            if in_deps:
                header = line.strip().strip("[]")
                if header.count(".") and not header.endswith("dependencies"):
                    names.append(header.rsplit(".", 1)[1])
            continue
        if in_deps:
            m = DEP_LINE.match(line)
            if m:
                names.append(m.group(1))
    return names


def main() -> int:
    root = Path(sys.argv[1] if len(sys.argv) > 1 else ".")
    checked = 0
    offences: list[str] = []

    for crate, (forbidden, why) in RULES.items():
        manifest = root / crate / "Cargo.toml"
        if not manifest.is_file():
            print(f"NOTHING WAS READ: {crate}/Cargo.toml is missing", file=sys.stderr)
            return 2
        checked += 1
        for dep in declared_dependencies(manifest.read_text()):
            if any(dep.startswith(p) for p in forbidden):
                offences.append(f"{crate}: depends on {dep} - {why}")

    if checked != len(RULES):
        print("NOTHING WAS READ: no manifest was checked", file=sys.stderr)
        return 2

    if offences:
        print("dependency direction broken:", file=sys.stderr)
        for o in offences:
            print(f"  {o}", file=sys.stderr)
        print(
            "\nThe crate's own module doc states this rule. If the rule has changed,"
            "\nchange it there first - this file follows the code, not the reverse.",
            file=sys.stderr,
        )
        return 1

    print(f"check-dependency-direction: {checked} crates hold the layer rule they state")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
