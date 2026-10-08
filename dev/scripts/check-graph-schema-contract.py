#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""The AI's view of the graph schema names everything the graph declares.

`contracts/graph-schema` is what the structured query layer validates against
and what `describe_schema` tells a model the graph holds. It is a hand mirror of
the DDL in `daemons/knowledge/src/graph.rs`, and on 8 October it listed 14 of 22
node tables and 11 of 26 relationship tables: a model could not ask about a
`Command`, a `Meeting` or a `CO_ACCESSED` pair, because the layer in front of the
graph refused a label it had never been told about. Nothing failed; the question
simply came back empty.

Checked, in both directions:
  - every node and relationship table in the DDL is in the contract, or named in
    `EXCLUDED` below with the reason it is kept from the AI;
  - every contract entry exists in the DDL;
  - every DDL column of a listed node is listed, and the other way round;
  - a relationship's endpoints agree;
  - a column's type agrees wherever the DDL spells it out.

Usage: check-graph-schema-contract.py [REPO_ROOT]
"""

from __future__ import annotations

try:
    import tracked_walk  # noqa: F401  the walk below reads what git knows about
except ModuleNotFoundError:  # a control's copy of this gate, run away from the module
    pass
import importlib.util
import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
DDL = ROOT / "daemons/knowledge/src/graph.rs"
CONTRACT = ROOT / "contracts/graph-schema/src/lib.rs"

#: Tables the DDL declares and the AI is deliberately not told about.
EXCLUDED = {
    "Grant": "the authority projection: the general read path denies it, access_grants (0x05) is its only reader",
    "CapabilityUse": "the authority projection, as Grant",
    "EntityType": "the authority projection, as Grant",
    "GRANTS": "the authority projection, as Grant",
    "USED_BY": "the authority projection, as Grant",
    "LAST_EXERCISED": "the authority projection, as Grant",
    "MergeSuggestion": "the shared-entity dedup queue, internal to the merge flow; its DOUBLE score has no type in the query layer",
}

TYPES = {"STRING": "Text", "INT64": "Int", "BOOLEAN": "Bool", "BOOL": "Bool"}


def _sibling():
    """`check-graph-columns.py`, which already reads the DDL's tables and endpoints."""
    path = Path(__file__).resolve().parent / "check-graph-columns.py"
    spec = importlib.util.spec_from_file_location("check_graph_columns", path)
    mod = importlib.util.module_from_spec(spec)
    argv, sys.argv = sys.argv, [str(path), str(ROOT)]
    try:
        spec.loader.exec_module(mod)
    finally:
        sys.argv = argv
    return mod


def ddl_types(text: str) -> dict[tuple[str, str], str]:
    """(table, column) -> contract type, wherever the DDL writes the type out."""
    out: dict[tuple[str, str], str] = {}
    for m in re.finditer(r"CREATE NODE TABLE IF NOT EXISTS (\w+)\((.*?)PRIMARY KEY", text, re.S):
        for col, ty in re.findall(r"(\w+)\s+([A-Z][A-Z0-9]*)\s*,", m.group(2)):
            out[(m.group(1), col)] = TYPES.get(ty, ty)
    for t, col, ty in re.findall(r"ALTER TABLE (\w+) ADD IF NOT EXISTS (\w+) ([A-Z][A-Z0-9]*)", text):
        out[(t, col)] = TYPES.get(ty, ty)
    return out


def contract(text: str):
    nodes = {}
    for m in re.finditer(r'NodeSchema \{\s*label: "(\w+)",\s*fields: &\[(.*?)\],\s*\}', text, re.S):
        nodes[m.group(1)] = dict(re.findall(r'\("(\w+)", FieldType::(\w+)\)', m.group(2)))
    edges = {
        m.group(1): (m.group(2), m.group(3))
        for m in re.finditer(r'EdgeSchema \{\s*label: "(\w+)",\s*from: "(\w+)",\s*to: "(\w+)"', text)
    }
    return nodes, edges


def main() -> int:
    if not DDL.is_file() or not CONTRACT.is_file():
        print(f"refusing: {DDL} or {CONTRACT} is missing, so nothing was compared")
        return 2
    ddl_text = DDL.read_text(encoding="utf-8")
    gc = _sibling()
    tables, _ = gc.declared_tables(ddl_text)
    endpoints = gc.declared_endpoints(ddl_text)
    node_tables = {t: cols for t, cols in tables.items() if t not in endpoints}
    types = ddl_types(ddl_text)
    nodes, edges = contract(CONTRACT.read_text(encoding="utf-8"))
    if not node_tables or not nodes:
        print("refusing: read no tables from the DDL or no entries from the contract")
        return 2

    problems = []
    for t in sorted(set(node_tables) | set(endpoints)):
        if t in EXCLUDED:
            if t in nodes or t in edges:
                problems.append(f"{t} is excluded ({EXCLUDED[t]}) and listed in the contract anyway")
            continue
        if t not in nodes and t not in edges:
            problems.append(f"{t} is declared in graph.rs and missing from the contract")
    for name in sorted(set(nodes) | set(edges)):
        if name not in node_tables and name not in endpoints:
            problems.append(f"{name} is in the contract and graph.rs declares no such table")
    for name in sorted(set(EXCLUDED) - set(node_tables) - set(endpoints)):
        problems.append(f"{name} is excluded but graph.rs no longer declares it; drop the exclusion")
    for n, fields in sorted(nodes.items()):
        if n not in node_tables:
            continue
        for col in sorted(node_tables[n] - set(fields)):
            problems.append(f"{n}.{col} is a column in graph.rs and missing from the contract")
        for col in sorted(set(fields) - node_tables[n]):
            problems.append(f"{n}.{col} is in the contract and graph.rs declares no such column")
        for col, ty in sorted(fields.items()):
            want = types.get((n, col))
            if want and want != ty:
                problems.append(f"{n}.{col} is {ty} in the contract and {want} in graph.rs")
    for e, (a, b) in sorted(edges.items()):
        if e in endpoints and endpoints[e] != (a, b):
            problems.append(f"{e} runs {a} -> {b} in the contract and {endpoints[e][0]} -> {endpoints[e][1]} in graph.rs")

    if problems:
        print("contracts/graph-schema disagrees with daemons/knowledge/src/graph.rs:")
        for p in problems:
            print(f"  {p}")
        return 1
    print(
        f"OK: the contract lists {len(nodes)} node and {len(edges)} relationship tables, "
        f"{len(EXCLUDED)} excluded by name"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
