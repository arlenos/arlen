#!/usr/bin/env python3
"""Every check must have a control.

A check that has only ever passed is an assertion, not a control: nothing has
shown it can still fail. This repository has learned that the hard way more than
once - a lint whose default pointed at `dev/*-baseline.tsvv`, one v too many, so a
bare run compared against a file that does not exist and reported success; a
`window_top` that answered a modal's border because its threshold could not see a
window edge; and three controls written on 5 September that passed over fixtures
their gate had never looked at.

So the rule is structural rather than a matter of remembering: a `check-*` in
`dev/scripts` has a `test-check-*` beside it. On 5 September that became true for
all of them, and this is what keeps it true - the property is easy to hold and
easy to lose, since the moment to skip the control is exactly the moment a new
check feels obviously right.

WHAT THIS CANNOT SAY, and it is most of what matters: whether the control drives
a real fault. A file named `test-check-x` that asserts nothing passes here. That
is a person's judgement, and the three vacuous ones this week were all caught by
reading, not by a rule. What this stops is the case with no control at all, which
is the one nobody notices.
"""

import pathlib
import sys

ROOT = pathlib.Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else pathlib.Path(__file__).resolve().parents[2]
SCRIPTS = ROOT / "dev" / "scripts"


def controls_for(check: pathlib.Path) -> list[pathlib.Path]:
    """The control files that would belong to a check, whether or not they exist."""
    stem = check.stem
    return [SCRIPTS / f"test-{stem}.mjs", SCRIPTS / f"test-{stem}.py"]


def main() -> int:
    """Refuse a check that nothing has shown can fail."""
    if not SCRIPTS.is_dir():
        print(f"check-controls-exist: no {SCRIPTS}, nothing to check")
        return 1
    checks = sorted(
        p for p in SCRIPTS.iterdir()
        if p.name.startswith("check-") and p.suffix in {".py", ".sh", ".mjs"}
    )
    missing = [c for c in checks if not any(p.exists() for p in controls_for(c))]
    for c in missing:
        print(
            f"{c.name} has no control. Write `test-{c.stem}.mjs` beside it: put the "
            f"fault in, run the check, require it to fail naming the fault, then take "
            f"the fault out and require it to pass. Run it over a fixture rather than "
            f"this tree - the gates run concurrently, so a control that edits tracked "
            f"files breaks its neighbours."
        )
    if missing:
        print(f"\n{len(missing)} of {len(checks)} check(s) have nothing proving they still fail")
        return 1
    print(
        f"{len(checks)} check(s), each with a control beside it. Presence only: whether "
        f"a control drives a real fault is a person's reading, and a control that "
        f"asserts nothing passes this."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
