#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A struct nested inside a `camelCase` one must agree with it.

`#[serde(rename_all = "camelCase")]` applies to ONE struct. It is not inherited,
so a nested struct without it ships `per_core` inside a payload whose every other
key is camelCase, and the consumer reading `perCore` gets `undefined`.

That happened on 18 August. `SystemTick` carried the attribute, `LoadAverage` did
not, and the system monitor's load line simply did not appear - no error, no
warning, a sentence missing from a pane nobody had reason to stare at. Finding it
took a build and a screenshot.

WHY A TYPE CHECK CANNOT DO THIS. The frontend's TypeScript said `perCore` and so
did the Rust field name; `svelte-check` passed with zero errors and could not
have done otherwise. TypeScript checks a hand-written interface against its own
uses, never against what a Rust process actually puts on the wire, so the two
sides can disagree in silence indefinitely.

WHAT THIS DOES NOT CLAIM. Plenty of `Serialize` structs are snake_case on
purpose - anything read from or written to a TOML file, and every wire type whose
consumer expects snake_case. This is not a campaign for camelCase. It only asks
that a struct REACHED FROM a camelCase one does not contradict it, which is a
question about a single payload rather than about the tree's style.

Run: dev/scripts/check-serde-nesting.py [repo-root]
"""

import pathlib
import re
import sys

ROOT = (
    pathlib.Path(sys.argv[1]).resolve()
    if len(sys.argv) > 1
    else pathlib.Path(__file__).resolve().parents[2]
)

CRATE_ROOTS = ("apps", "daemons", "sdk", "contracts", "forage", "ai", "store-backend", "dev")

#: `#[derive(...)] pub struct Name { ... }`, attributes captured so the derive and
#: any serde attribute can be read off them.
STRUCT = re.compile(r"((?:#\[[^\]]*\]\s*)+)pub struct (\w+)\s*\{([^}]*)\}")
#: A field with the attributes immediately above it, so a per-field `rename` is
#: read where it applies rather than anywhere in the struct. The first cut looked
#: for `rename = "<field name>"` in the whole body, which never matches: the
#: rename's VALUE is the new name, not the old one.
FIELD = re.compile(r"((?:#\[[^\]]*\]\s*)*)pub (\w+)\s*:\s*([^,\n]+)")
CAMEL = 'rename_all = "camelCase"'


def collect(root: pathlib.Path) -> dict:
    """Every `Serialize` struct in the tree, by name."""
    out: dict[str, dict] = {}
    for crate in CRATE_ROOTS:
        for path in (root / crate).rglob("*.rs"):
            if "target" in path.parts or "node_modules" in path.parts:
                continue
            if "mkosi.builddir" in str(path):
                continue
            try:
                text = path.read_text()
            except OSError:
                continue
            for m in STRUCT.finditer(text):
                attrs, name, body = m.group(1), m.group(2), m.group(3)
                if "Serialize" not in attrs:
                    continue
                fields = FIELD.findall(body)
                out[name] = {
                    "file": path.relative_to(root),
                    "camel": CAMEL in attrs,
                    # A field with its own `rename` has said what it wants and is
                    # nobody's problem.
                    "snake": [
                        n for fattrs, n, _ in fields if "_" in n and "rename" not in fattrs
                    ],
                    "types": [ty for _, _, ty in fields],
                }
    return out


def main() -> int:
    structs = collect(ROOT)
    if not structs:
        print(f"NOTHING WAS READ: no Serialize struct under {CRATE_ROOTS}", file=sys.stderr)
        return 2

    findings = []
    for name, s in structs.items():
        if not s["camel"]:
            continue
        for ty in s["types"]:
            # `Option<Foo>`, `Vec<Foo>`, `Foo` all name Foo.
            for inner in re.findall(r"\b([A-Z]\w+)\b", ty):
                if inner == name or inner not in structs:
                    continue
                other = structs[inner]
                if other["camel"] or not other["snake"]:
                    continue
                findings.append(
                    f"{s['file']}: `{name}` is camelCase and carries `{inner}` "
                    f"({other['file']}), which is not - so "
                    f"{', '.join('`' + f + '`' for f in other['snake'][:3])} "
                    f"ship snake_case inside a camelCase payload."
                )

    if findings:
        print(
            f"{len(structs)} Serialize struct(s) read, {len(findings)} nested "
            f"disagreement(s):\n",
            file=sys.stderr,
        )
        for f in sorted(set(findings)):
            print(f"  - {f}", file=sys.stderr)
        print(
            "\nAdd `#[serde(rename_all = \"camelCase\")]` to the nested struct, or "
            "an explicit `rename` per field. serde does not inherit it, and the "
            "consumer reads `undefined` rather than failing.",
            file=sys.stderr,
        )
        return 1

    camel = sum(1 for s in structs.values() if s["camel"])
    print(
        f"{len(structs)} Serialize struct(s) read, {camel} of them camelCase: every "
        f"struct nested inside one agrees with it. serde applies the attribute per "
        f"struct, so a nested one that forgets it goes out in the wrong case and "
        f"the consumer sees a missing field rather than an error."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
