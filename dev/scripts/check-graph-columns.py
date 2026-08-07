# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that every Cypher query names columns the graph schema declares.

A query is a string in one crate, checked against a schema declared in another
(`daemons/knowledge/src/graph.rs`). Nothing joins the two until a socket round
trip at runtime, and the engine's answer to a column that does not exist is not
a crash but an error the caller usually swallows:

    Binder exception: Cannot find property expired_at for f.

That is how a member listing in the Knowledge app died on 7 August. It compiled,
its tests passed, and the app fell back to fixture data - which a user reads as
"the graph knows nothing about this project" rather than "the query was wrong".
Silent and wrong beats loud and wrong for exactly one audience, and it is not
the user.

This is `check-invoke-shape` for the graph boundary: a fact in one file compared
against a fact in another.

**Only labels the schema declares are checked.** The entity registry creates
tables at runtime, so an unknown label means "not declared here", never "wrong" -
guessing there would make this a false-positive machine, and a check people
learn to ignore is worse than no check.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
SCHEMA = ROOT / "daemons/knowledge/src/graph.rs"

# Directories whose Cypher is not ours to check: vendored code and build output.
SKIP_PARTS = {"target", "node_modules", ".git", "build"}

# Aliases bound to something this checker cannot resolve to one label - a
# variable-length path, a label-less pattern - are skipped rather than guessed.
BIND_NODE = re.compile(r"\(\s*(\w+)\s*:\s*(\w+)")
BIND_REL = re.compile(r"\[\s*(\w+)\s*:\s*(\w+)")
# `alias.column`, but not `alias.column(` (a function call on a namespace) and
# not a decimal in a literal.
REF = re.compile(r"\b([a-zA-Z_]\w*)\.([a-zA-Z_]\w*)\b(?!\s*\()")

# Rust string literals, including the escaped-newline continuations this tree
# uses to lay a query out over several lines.
STRING = re.compile(r'"((?:[^"\\]|\\.)*)"', re.S)

CYPHER_MARKERS = ("MATCH (", "MERGE (", "CREATE (", "OPTIONAL MATCH (")

# Words that look like an alias.column reference but are not: Rust method calls
# and module paths that happen to sit inside a query string are impossible here
# (the string is Cypher), but a Cypher function namespace is not.
NOT_A_COLUMN = {"count", "collect", "size", "labels", "properties"}

# `(a:A)-[r:REL]->(b:B)` and its mirror. The node parts are matched loosely
# because they carry property maps; the label is read out of them afterwards.
PATTERN = re.compile(
    r"\(([^()]*)\)\s*(<-|-)\s*\[([^\]]*)\]\s*(->|-)\s*\(([^()]*)\)"
)
LABEL_IN = re.compile(r":\s*(\w+)")


def declared_tables(text: str) -> tuple[dict[str, set[str]], set[str]]:
    """Every table the schema declares with its columns, and the tables whose
    column list this parser could not fully resolve.

    The second set is the honest half. A table built in a way the parser cannot
    follow has an INCOMPLETE column set here, and checking against an incomplete
    set reports working code as broken - so those tables are skipped and said
    aloud rather than half-checked.
    """
    tables: dict[str, set[str]] = {}
    unresolved: set[str] = set()

    for m in re.finditer(
        r"CREATE NODE TABLE IF NOT EXISTS\s+(\w+)\s*\((.*?)PRIMARY KEY", text, re.S
    ):
        tables[m.group(1)] = _columns(m.group(2))

    for m in re.finditer(r"CREATE REL TABLE IF NOT EXISTS\s+(\w+)\s*\((.*?)\)", text, re.S):
        body = m.group(2)
        # `FROM A TO B` is the endpoint declaration, not a column.
        body = re.sub(r"FROM\s+\w+\s+TO\s+\w+,?", "", body)
        tables[m.group(1)] = _columns(body)

    # Additive evolution. A table can gain a column long after its CREATE, and a
    # query written against the newer schema is correct - so these count.
    for m in re.finditer(r"ALTER TABLE\s+(\w+)\s+ADD IF NOT EXISTS\s+(\w+)", text):
        tables.setdefault(m.group(1), set()).add(m.group(2))

    # The same statement built in a loop over a literal column list:
    #
    #     for column in ["valid_at INT64", "invalid_at INT64", ...] {
    #         conn.query(&format!("ALTER TABLE FILE_PART_OF ADD IF NOT EXISTS {column}"))
    #
    # Eleven of FILE_PART_OF's columns arrive this way, including every
    # bitemporal stamp, so a parser that cannot follow it reports the most-queried
    # table in the tree as having almost no columns - which is how the first cut
    # of this check produced 28 findings, all wrong, against code that works.
    for m in re.finditer(
        r"ALTER TABLE\s+(\w+)\s+ADD IF NOT EXISTS\s+\{(\w+)\}", text
    ):
        table, var = m.group(1), m.group(2)
        loop = None
        for cand in re.finditer(rf"for\s+{re.escape(var)}\s+in\s+\[(.*?)\]", text, re.S):
            if cand.start() < m.start():
                loop = cand  # the nearest preceding loop binding this variable
        if loop is None:
            unresolved.add(table)
            continue
        for lit in re.findall(r'"([^"]*)"', loop.group(1)):
            name = lit.strip().split()
            if name and name[0].isidentifier():
                tables.setdefault(table, set()).add(name[0])

    return tables, unresolved


def declared_endpoints(text: str) -> dict[str, tuple[str, str]]:
    """Each relationship table's declared `FROM A TO B`.

    Direction is a fact the schema states and a query can get backwards. The
    engine does not complain: it matches nothing and returns an empty result,
    which reads exactly like "there is no such membership" - the same silence a
    missing column produces, one layer up.
    """
    out = {}
    for m in re.finditer(
        r"CREATE REL TABLE IF NOT EXISTS\s+(\w+)\s*\(\s*FROM\s+(\w+)\s+TO\s+(\w+)", text
    ):
        out[m.group(1)] = (m.group(2), m.group(3))
    return out


def misdirected(query: str, endpoints: dict[str, tuple[str, str]]) -> list[tuple[str, str, str]]:
    """(rel_type, left_label, right_label) for each traversal written against
    the declared direction.

    Only a pattern whose BOTH endpoints carry a label is judged. An undirected
    `-[r:X]-` is deliberate wherever it appears (a neighbour walk wants both
    ways), so it is not a finding.
    """
    found = []
    for m in PATTERN.finditer(query):
        left_part, left_arrow, rel_part, right_arrow, right_part = m.groups()
        rel = LABEL_IN.search(rel_part)
        left = LABEL_IN.search(left_part)
        right = LABEL_IN.search(right_part)
        if not (rel and left and right):
            continue
        declared = endpoints.get(rel.group(1))
        if declared is None:
            continue
        if left_arrow == "-" and right_arrow == "->":
            actual = (left.group(1), right.group(1))
        elif left_arrow == "<-" and right_arrow == "-":
            actual = (right.group(1), left.group(1))
        else:
            continue  # undirected, and meant to be
        if actual != declared:
            found.append((rel.group(1), actual[0], actual[1]))
    return found


def _columns(body: str) -> set[str]:
    """Column names from a table body: the first word of each comma-separated
    declaration."""
    out = set()
    for part in body.split(","):
        part = part.strip()
        if not part:
            continue
        name = part.split()[0]
        if name.isidentifier():
            out.add(name)
    return out


def _all_queries(text: str) -> list[str]:
    """Every Cypher-looking literal, test module included - used only to say how
    many the test-module cut left unchecked."""
    return [
        m.group(1)
        for m in STRING.finditer(text)
        if any(k in m.group(1) for k in CYPHER_MARKERS)
    ]


def queries(text: str) -> list[str]:
    """The Cypher-looking string literals in one Rust file, EXCLUDING its test
    module.

    A test's Cypher is illustrative - an expected-output assertion for a query
    builder, an input to a lexer - and names whatever the test needs, including
    columns no table has. Checking those reports a passing test as a broken
    query. The cut is at the first `#[cfg(test)]`, which in this tree is the
    trailing test module; a file that put one in the middle would have its later
    queries skipped, so the count of truncated files is printed rather than left
    implicit.
    """
    cut = text.find("#[cfg(test)]")
    if cut != -1:
        text = text[:cut]
    return [
        m.group(1)
        for m in STRING.finditer(text)
        if any(k in m.group(1) for k in CYPHER_MARKERS)
    ]


def offenders(query: str, tables: dict[str, set[str]]) -> list[tuple[str, str, str]]:
    """(alias, label, column) for each reference to a column its label lacks.

    Scoped to ONE query string on purpose: the same alias means different things
    in two queries in the same file, and merging them would invent errors.
    """
    bound: dict[str, str] = {}
    for m in BIND_NODE.finditer(query):
        bound[m.group(1)] = m.group(2)
    for m in BIND_REL.finditer(query):
        bound[m.group(1)] = m.group(2)

    found = []
    for m in REF.finditer(query):
        alias, column = m.group(1), m.group(2)
        if column in NOT_A_COLUMN:
            continue
        label = bound.get(alias)
        # An alias bound to nothing, or to a label this schema does not declare
        # (the runtime entity registry owns those), is not this check's business.
        if label is None or label not in tables:
            continue
        if column not in tables[label]:
            found.append((alias, label, column))
    return found


def main() -> int:
    text = SCHEMA.read_text()
    tables, unresolved = declared_tables(text)
    endpoints = declared_endpoints(text)
    for table in unresolved:
        tables.pop(table, None)
    if not tables:
        print(f"no table declarations found in {SCHEMA.relative_to(ROOT)};")
        print("the schema moved or its syntax changed - fix this check rather than")
        print("letting it pass on an empty schema, which would check nothing.")
        return 1

    problems: list[str] = []
    scanned = truncated = 0
    for path in ROOT.rglob("*.rs"):
        if SKIP_PARTS & set(path.parts):
            continue
        try:
            body = path.read_text()
        except (UnicodeDecodeError, OSError):
            continue
        # Count a file as truncated only when its test module actually held
        # Cypher - otherwise the number counts every file in the tree that has a
        # test module at all, which says nothing about coverage.
        skipped_here = len(_all_queries(body)) - len(queries(body))
        if skipped_here > 0:
            truncated += skipped_here
        for q in queries(body):
            scanned += 1
            bad = offenders(q, tables)
            for rel, left, right in misdirected(q, endpoints):
                a, b = endpoints[rel]
                problems.append(
                    f"{path.relative_to(ROOT)}: `{left}-[:{rel}]->{right}` runs "
                    f"against the declared direction ({a} to {b}), so it matches "
                    f"nothing and the caller reads an empty result as absence"
                )
            for alias, label, column in bad:
                problems.append(
                    f"{path.relative_to(ROOT)}: `{alias}.{column}` - the {label} "
                    f"table has no {column} column, so this query fails at the "
                    f"binder and the caller sees an error, not rows"
                )

    if problems:
        print("queries that disagree with the graph schema:\n")
        for p in sorted(set(problems)):
            print(f"  - {p}")
        return 1

    print(
        f"{len(tables)} declared table(s), {len(endpoints)} of them directed, "
        f"{scanned} cypher literal(s) scanned: every labelled reference names a "
        f"declared column and every directed traversal follows the schema"
    )
    print(
        f"{truncated} cypher literal(s) inside test modules were not checked"
    )
    if unresolved:
        print(
            "not checked, because this parser could not resolve their columns: "
            + ", ".join(sorted(unresolved))
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
