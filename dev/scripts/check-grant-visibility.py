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

Two lists qualify it. `NOT_REACH` is for a field that grants nothing - a
narrowing, an assertion. `REACH_NOT_YET_SUMMARISED` is the opposite and is a
ledger rather than an exemption: real reach that a person cannot currently see,
carried in the output so the number is visible instead of resting in a comment.
Moving an entry out of it is the work; adding one is admitting a gap.
"""

import pathlib
import re
import sys

# The tree to scan. An argument so this can be pointed at a fixture and shown
# to fail: a check that only ever runs against a tree that already passes
# cannot demonstrate the defect it exists for (standing rule, 11 Aug).
ROOT = (
    pathlib.Path(sys.argv[1]).resolve()
    if len(sys.argv) > 1
    else pathlib.Path(__file__).resolve().parents[2]
)
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
    # The three GraphPermissions fields its own summary classifies as not-reach,
    # quoted from the comments there so the two cannot drift apart silently.
    ("GraphPermissions", "app_isolated"): "a narrowing, not reach",
    ("GraphPermissions", "instance_scope"): "which instances, not which types",
    ("GraphPermissions", "required"): "an install-time assertion, not reach",
    ("McpPermissions", "always_confirm_overrides"): (
        "narrows rather than grants: it marks extra tools always-confirm, so an "
        "app can only make itself MORE interruptive with it, never reach further"
    ),
}

# (struct, field) -> reach that is real and does not reach a summary yet. This is
# a LEDGER, not an excuse: each entry is a grant a person cannot currently see on
# the App-access page. `GraphPermissions::reach_summary` already classifies its
# own fields in comments and marks these three "Reach, not yet summarised"; this
# repeats the classification where a check can count it, so the number appears in
# the output instead of living only in a comment nobody greps.
REACH_NOT_YET_SUMMARISED = {
    ("GraphPermissions", "annotations_read_cross_namespace"): (
        "reading another app's annotations is an explicit grant (foundation §395)"
    ),
    ("GraphPermissions", "relations"): "edge-write grants",
    ("GraphPermissions", "delegated_namespaces"): (
        "authority handed to another principal"
    ),
    ("GraphPermissions", "read_sensitive"): (
        "gated separately at the daemon, which settles ENFORCEMENT and not "
        "visibility - reading fields the profile marks sensitive is reach by any "
        "reading of the word, and it appears in no summary and in no token scope "
        "(EntityScope carries entity_type/fields/exclude_fields, no sensitive flag)"
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
                and (name, f) not in REACH_NOT_YET_SUMMARISED
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
            if not re.search(rf"\b{f}\b", body)
            and (name, f) not in NOT_REACH
            and (name, f) not in REACH_NOT_YET_SUMMARISED
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
    open_gap = len(REACH_NOT_YET_SUMMARISED)
    print(
        f"{len(declared)} permission dimension(s), {total} declared grant(s), "
        f"{total - open_gap} reach a summary"
    )
    print(f"{open_gap} are reach that does not reach one yet, each named in this file:")
    for (struct, field), why in sorted(REACH_NOT_YET_SUMMARISED.items()):
        print(f"  - {struct}.{field}: {why}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
