# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that the copies of a shared .proto agree on field numbers.

`event.proto` exists in eight places, because there is no crate for it yet and
each daemon that decodes Event Bus traffic vendors its own copy. That is recorded
as a TODO in the notification daemon's build.rs and is fine as far as it goes:
protobuf tolerates a copy that is missing a field, since an absent field decodes
to its default.

What it does not tolerate is two copies giving the SAME field two different
numbers, or reusing a number for a different field. Either one is a silent
mis-decode across a socket rather than a build error, and nothing here would say
so, because the copies are only ever compiled separately.

So this compares them pairwise: for every message that appears in more than one
copy, every field they share must have the same number, and no number may be
used for two different field names. Divergence by absence stays legal, which is
what lets a copy lag a field like `cgroup_id` without failing.
"""

import collections
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]

PACKAGE = re.compile(r"^\s*package\s+([\w.]+)\s*;", re.M)
MESSAGE = re.compile(r"^\s*message\s+(\w+)\s*\{", re.M)
FIELD = re.compile(r"^\s*(?:repeated\s+|optional\s+)?[\w.]+\s+(\w+)\s*=\s*(\d+)\s*;", re.M)


def messages(text: str) -> dict[str, dict[str, int]]:
    """Field name to number, per message. Nested braces are not used here."""
    out: dict[str, dict[str, int]] = {}
    for m in MESSAGE.finditer(text):
        start = m.end()
        depth = 1
        i = start
        while i < len(text) and depth:
            if text[i] == "{":
                depth += 1
            elif text[i] == "}":
                depth -= 1
            i += 1
        body = text[start : i - 1]
        out[m.group(1)] = {f.group(1): int(f.group(2)) for f in FIELD.finditer(body)}
    return out


def main() -> int:
    files = subprocess.run(
        ["git", "ls-files", "*.proto"], cwd=ROOT, capture_output=True, text=True, check=True
    ).stdout.split()

    # A scan that found nothing cannot report agreement. `git ls-files` returning
    # empty is not a repo without protos, it is a discovery that broke - a rename,
    # a move out of the index, a wrong working directory - and the summary line
    # below would have said "0 proto file(s) ... no disagreement" and exited 0.
    # Eight copies of the event wire format agreeing is the whole point of this
    # check; claiming it over nothing is the one answer it must never give.
    if not files:
        print("found no .proto files; the check needs updating")
        return 2

    # message -> field -> number -> the files that say so
    seen: dict[str, dict[str, dict[int, list[str]]]] = collections.defaultdict(
        lambda: collections.defaultdict(lambda: collections.defaultdict(list))
    )
    # message -> number -> field names, to catch a reused number
    by_number: dict[str, dict[int, set[str]]] = collections.defaultdict(
        lambda: collections.defaultdict(set)
    )

    for f in files:
        text = (ROOT / f).read_text()
        # Keyed by package, not by message name alone: `HistoryResponse` exists in
        # both `arlen.clipboard` and `arlen.notification` with different field 1s,
        # and those are two unrelated messages rather than copies that disagree.
        pkg = PACKAGE.search(text)
        pkg = pkg.group(1) if pkg else "(no package)"
        for message_name, fields in messages(text).items():
            msg = f"{pkg}.{message_name}"
            for field_name, num in fields.items():
                seen[msg][field_name][num].append(f)
                by_number[msg][num].add(field_name)

    problems: list[str] = []
    for msg, fields in sorted(seen.items()):
        for name, numbers in sorted(fields.items()):
            if len(numbers) > 1:
                where = "; ".join(
                    f"{n} in {', '.join(sorted(fs))}" for n, fs in sorted(numbers.items())
                )
                problems.append(f"{msg}.{name} has two numbers: {where}")
        for num, names in sorted(by_number[msg].items()):
            if len(names) > 1:
                problems.append(f"{msg} reuses field number {num} for {sorted(names)}")

    if problems:
        print("proto copies disagree:\n")
        for p in problems:
            print(f"  - {p}")
        print("\na field's number is part of the wire format; copies must agree on it")
        return 1

    if not seen:
        print("found .proto files but no messages in them; the check needs updating")
        return 2

    shared = sum(1 for m, fs in seen.items() for f in fs.values() if len(f) >= 1)
    print(f"{len(files)} proto file(s), {len(seen)} message(s), {shared} field(s), no disagreement")
    return 0


if __name__ == "__main__":
    sys.exit(main())
