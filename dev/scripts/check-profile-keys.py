# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that every key in a permission profile is a key the schema has.

Written on 25 August after making the defect. Staging the pdf reader I gave it
`read = [...]` under `[filesystem]`, which is not a field. `FilesystemPermissions`
derives every field with `#[serde(default)]` and no `deny_unknown_fields`, so the
section parsed as EMPTY: the reader would have launched confined with no path at
all, while the file read like a considered set of grants with a paragraph of
reasoning above it.

WHY A GATE AND NOT A PARSER CHANGE. Making the daemon reject an unknown key is a
runtime change to a security-critical parser with a real trade behind it (a profile
written for a newer schema would stop loading on an older binary), so it is
somebody's decision rather than a coder's. A typo in a profile that ships WITH the
image is a build-time fact, and this is where it belongs. If the parser is tightened
later this gate stays useful, because it says WHICH key and WHERE.

WHAT IT CANNOT SEE. The schema is read out of the Rust source, so this knows what
`sdk/permissions/src/lib.rs` declares and nothing about what the code then does with
it. Sub-tables inside a section (`[[graph.relations]]` and its like) are skipped
rather than guessed at: their fields live in types this does not resolve, and a
check that half-knows a shape reports confident nonsense. The extraction failing to
find the schema at all is an ERROR, not an empty pass - a gate that reads nothing
must never look like a gate that found nothing wrong.

Run: dev/scripts/check-profile-keys.py [root]
"""

from __future__ import annotations

import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

#: The schema, as the SDK declares it.
SCHEMA = ROOT / "sdk/permissions/src/lib.rs"

#: Every profile that ships on the image, under the uid directory it applies to.
PROFILE_DIRS = (ROOT / "dev/mkosi/mkosi.extra/var/lib/arlen/permissions",)

STRUCT = re.compile(r"pub struct (\w+)\s*\{(.*?)\n\}", re.S)
FIELD = re.compile(r'(?:#\[serde\(rename\s*=\s*"([^"]+)"\)\]\s*)?pub (\w+)\s*:\s*([^,\n]+)', re.S)


def struct_fields(source: str) -> dict[str, list[tuple[str, str]]]:
    """Every `pub struct` in `source`, as name -> [(wire key, declared type)]."""
    out: dict[str, list[tuple[str, str]]] = {}
    for name, body in STRUCT.findall(source):
        fields = []
        for renamed, field, ty in FIELD.findall(body):
            fields.append((renamed or field, ty.strip()))
        out[name] = fields
    return out


def main() -> int:
    if not SCHEMA.is_file():
        print(f"NOTHING WAS READ: {SCHEMA} is missing, so no key was checked", file=sys.stderr)
        return 2

    structs = struct_fields(SCHEMA.read_text(encoding="utf-8"))
    profile = structs.get("PermissionProfile")
    if not profile:
        print(
            "NOTHING WAS READ: `PermissionProfile` was not found in the schema, so the "
            "section list is unknown and nothing could be checked",
            file=sys.stderr,
        )
        return 2

    # section name -> the keys that section accepts. A section whose type this
    # cannot resolve is left out and reported, never silently treated as open.
    sections: dict[str, set[str]] = {}
    unresolved: list[str] = []
    for key, ty in profile:
        fields = structs.get(ty.strip())
        if fields is None:
            unresolved.append(f"{key} (type {ty.strip()})")
            continue
        sections[key] = {f for f, _ in fields}

    if not sections:
        print(
            "NOTHING WAS READ: no profile section resolved to a struct, so every key "
            "would have passed",
            file=sys.stderr,
        )
        return 2

    problems: list[str] = []
    checked = 0
    for directory in PROFILE_DIRS:
        for path in sorted(directory.rglob("*.toml")):
            checked += 1
            try:
                data = tomllib.loads(path.read_text(encoding="utf-8"))
            except tomllib.TOMLDecodeError as e:
                problems.append(f"{path.relative_to(ROOT)}: not readable as TOML ({e})")
                continue
            for section, body in data.items():
                if section not in sections:
                    if section in unresolved:
                        continue
                    problems.append(
                        f"{path.relative_to(ROOT)}: `[{section}]` is not a section the "
                        f"schema has, so the whole table is ignored at load"
                    )
                    continue
                if not isinstance(body, dict):
                    continue
                for key, value in body.items():
                    # A sub-table's fields live in a type this does not resolve.
                    if isinstance(value, dict) or (
                        isinstance(value, list) and value and isinstance(value[0], dict)
                    ):
                        continue
                    if key not in sections[section]:
                        problems.append(
                            f"{path.relative_to(ROOT)}: `{key}` under `[{section}]` is not a "
                            f"field the schema has, so it is dropped at load and grants "
                            f"nothing. The section accepts: "
                            f"{', '.join(sorted(sections[section]))}"
                        )

    if not checked:
        print("NOTHING WAS READ: no profiles found, so this checked nothing", file=sys.stderr)
        return 2

    if problems:
        print("profiles carrying keys the schema does not have:\n")
        for p in problems:
            print(f"  - {p}")
        print(
            "\nA key the parser drops is worse than a missing one. The file reads\n"
            "like a grant somebody thought about, and the app gets nothing."
        )
        return 1

    resolved = len(sections)
    note = f", {len(unresolved)} section type(s) unresolved" if unresolved else ""
    print(
        f"check-profile-keys: {checked} profile(s) against {resolved} known section(s){note}; "
        f"every key is one the schema accepts."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
