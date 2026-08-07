# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that every declared grant reaches the summary a person reads.

A `*Permissions` dimension is projected into the capability graph through its
`reach_summary`, and that string is what the App-access page shows. A field the
summary does not mention is a grant nobody can see, and **a grant nobody can see
is a grant nobody can revoke** - which is the second, invisible store that
projecting profiles into the LCG (PAS-8) exists to prevent.

`emit_all_declared_grants` already destructures `PermissionProfile` with no rest
pattern, so a new *dimension* cannot arrive unprojected. This is the same
invariant one level down, where that guard does not reach: `schedule_wake` was
added to `PowerPermissions` on 7 August, compiled everywhere, and appeared in no
summary at all, because `SystemPermissions::reach_summary` bound `power` whole.
Nothing was red. It was found a day later by reading the projection while
checking something else.

The rule: every `pub` field of a `*Permissions` struct must be named in that
struct's `reach_summary`. A struct with no `reach_summary` of its own is
projected through a parent and is listed below with which one, so that being
absent is a decision someone wrote down rather than an omission.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
SOURCE = ROOT / "sdk/permissions/src/lib.rs"

# struct -> the parent whose `reach_summary` projects it. The parent must
# destructure it field-by-field, which is what keeps this list honest.
PROJECTED_BY_PARENT = {
    "PowerPermissions": "SystemPermissions",
}

# (struct, field) -> why it is absent from the summary. A field here is a claim
# that it is not reach, or that it is projected some other way. Anything else
# missing is a grant nobody can see.
NOT_REACH = {
    ("McpPermissions", "always_confirm_overrides"): (
        "narrows rather than grants: it marks extra tools always-confirm, so an "
        "app can only make itself MORE interruptive with it, never reach further"
    ),
}

# Structs whose grants are projected by a different mechanism entirely, with the
# one that does it. `emit_all_declared_grants` binds `graph: _` for this reason.
PROJECTED_ELSEWHERE = {
    "GraphPermissions": (
        "projected by lcg.rs::emit_grant_node from the minted capability token, "
        "not from the declared summary - the graph dimension is the token-based "
        "grant. NB that projection serialises the token's read/write/relations/"
        "instance scopes; whether read_sensitive, app_isolated, "
        "delegated_namespaces, annotations_read_cross_namespace and required "
        "reach it too is an open question, and a real one - read_sensitive in "
        "particular is reach by any reading of the word. Recorded here rather "
        "than answered, because the answer is in the token minting and deserves "
        "looking at rather than assuming"
    ),
}


def structs(text: str) -> dict[str, list[str]]:
    """Every `*Permissions` struct and its public fields."""
    out = {}
    for m in re.finditer(r"pub struct (\w+Permissions) \{(.*?)\n\}", text, re.S):
        out[m.group(1)] = re.findall(r"\n    pub (\w+):", m.group(2))
    return out


def summaries(text: str) -> dict[str, str]:
    """Every `reach_summary` body, by the type it belongs to, with its
    destructuring patterns removed.

    Removing them is the whole point. These functions open with
    `let Self { autostart, background, power } = self;`, which mentions every
    field by name - so a naive search for the name finds it whatever the body
    then does, and the check passes on a summary that projects nothing. Both
    mutation tests sailed through the first version for exactly that reason.
    What has to be searched is the part after the pattern, where a field is
    actually turned into words.
    """
    out = {}
    for m in re.finditer(r"impl (\w+Permissions) \{(.*?)\n\}\n", text, re.S):
        body = re.search(r"fn reach_summary\(&self\).*?\n    \}", m.group(2), re.S)
        if body:
            out[m.group(1)] = re.sub(r"let [^=]*\{[^}]*\}\s*=\s*\w+;", "", body.group(0), flags=re.S)
    return out


def main() -> int:
    text = SOURCE.read_text()
    declared, projected = structs(text), summaries(text)
    problems: list[str] = []

    for name, fields in sorted(declared.items()):
        if name in PROJECTED_ELSEWHERE:
            continue
        body = projected.get(name)
        if body is None:
            parent = PROJECTED_BY_PARENT.get(name)
            if parent is None:
                problems.append(
                    f"{name} has no reach_summary and no parent that projects it, so "
                    f"nothing it grants is visible. Give it one, or add it to "
                    f"PROJECTED_BY_PARENT with the parent that destructures it."
                )
                continue
            parent_body = projected.get(parent, "")
            missing = [
                f
                for f in fields
                if not re.search(rf"\b{f}\b", parent_body)
                and (name, f) not in NOT_REACH
            ]
            if missing:
                problems.append(
                    f"{name} is projected through {parent}, but {parent}'s summary "
                    f"never mentions {missing}. A grant that reaches no summary "
                    f"cannot be seen and so cannot be revoked."
                )
            continue
        missing = [
            f
            for f in fields
            if not re.search(rf"\b{f}\b", body) and (name, f) not in NOT_REACH
        ]
        if missing:
            problems.append(
                f"{name}::reach_summary does not mention {missing}, so those grants "
                f"are invisible on the App-access page. Name them, or say in the "
                f"summary why they are not reach."
            )

    if problems:
        print("declared grants that reach no summary:\n")
        for p in problems:
            print(f"  - {p}")
        return 1
    total = sum(len(f) for f in declared.values())
    print(
        f"{len(declared)} permission dimension(s), {total} declared grant(s), "
        f"every one of them reaches a summary"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
