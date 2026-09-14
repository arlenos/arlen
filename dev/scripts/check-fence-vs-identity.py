#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""A fenced daemon cannot identify its callers, so it must not try.

`fence_writes` installs a Landlock ruleset. Resolving a peer means reading
`/proc/<peer-pid>/exe`, which goes through `PTRACE_MODE_READ_FSCREDS` - and
`restrict_self` denies that for any process outside the domain. No read grant
lifts it, because the check is not about the path.

MEASURED, not reasoned. `arlen-capsuled` installs the fence and resolves its
peers, and the same nameable caller is admitted unfenced and refused fenced with
`cannot read exe path for pid N: Permission denied [failed at readlinkat(exe)]`.
Its entire serve path was dead in the configuration it ships, and the refusal
read like a policy decision rather than a broken one.

THE TREE ALREADY HELD THE ANSWER TWICE OVER, which is why this is a check and not
a note: `daemons/knowledge` is deliberately unfenced, with the reason in its
`main.rs` from a real image boot, and `arlen-timeline` keeps its fence precisely
because it does not read a peer's exe. Two daemons got the pairing right, one got
it wrong, and nothing compared them. The crate doc made it worse by listing
"`/proc` for caller-pid resolution" among the reads the fence leaves working; that
sentence is corrected now, and this is what keeps it corrected.

WHAT IS COMPARED. A component under `daemons/` that mentions `fence_writes` or
`arlen_landlock_fence`, against the same component mentioning
`ConnectionAuth::extract_from`, `path_to_app_id` or `app_id_from_pid`. Doing both
is the finding.

A daemon may need the fence AND a peer check - the way out is not to drop one but
to need less: SO_PEERCRED gives the uid and a pidfd gives liveness, and neither
touches `/proc/<pid>/exe`. That is a different call, so a known case sits in
CARRIED with its reason until somebody makes it.
"""

import sys
from pathlib import Path

OWN_TREE = len(sys.argv) <= 1
REPO = Path(__file__).resolve().parents[2] if OWN_TREE else Path(sys.argv[1]).resolve()

FENCES = ("fence_writes", "arlen_landlock_fence")
RESOLVES = ("ConnectionAuth::extract_from", "path_to_app_id", "app_id_from_pid")

#: Components known to do both, with the reason they have not been fixed yet.
#: This list must SHRINK. An entry is a decision somebody still owes, not a
#: permanent exemption.
CARRIED = {
    "daemons/capsuled": "found 14 September by starting it: the fence blocks the "
    "peer-exe read, so it admits nobody. The fix is a real fork - drop the fence, "
    "or stop resolving an identity it never uses (it calls only `verify_alive`, "
    "and builds its correlation id from the uid) - and the second needs a "
    "constructor `ConnectionAuth` does not have. Put to the planner in "
    "coder-reports.md; the refusal is at least loud now. AND THE FENCE IS THE "
    "ONLY THING MISSING: measured the same evening with the fence lifted and a "
    "nameable caller - a minted capsule is presented, verified, op-counted and "
    "SERVED, the slice comes back, the ledger records `capsule.read -> served`, "
    "and the fourth read against a max_ops of three is refused `exhausted`. So "
    "whichever way the fork goes, nothing else has to be built for this feature "
    "to work.",
}


def sources(component: Path) -> list[str]:
    out = []
    for f in (component / "src").rglob("*.rs"):
        if "/target/" in str(f):
            continue
        out.append(f.read_text(encoding="utf-8", errors="replace"))
    return out


def main() -> int:
    root = REPO / "daemons"
    if not root.is_dir():
        print(f"NOTHING WAS READ: no {root}", file=sys.stderr)
        return 2

    checked = 0
    findings: list[tuple[str, str]] = []
    carried_seen: set[str] = set()

    for component in sorted(p for p in root.iterdir() if (p / "src").is_dir()):
        texts = sources(component)
        if not texts:
            continue
        checked += 1
        blob = "\n".join(texts)
        fenced = next((m for m in FENCES if m in blob), None)
        resolved = next((m for m in RESOLVES if m in blob), None)
        if not (fenced and resolved):
            continue
        name = str(component.relative_to(REPO))
        if name in CARRIED:
            carried_seen.add(name)
            continue
        findings.append((name, f"fences with `{fenced}` and resolves with `{resolved}`"))

    if not checked:
        print("NOTHING WAS READ: no daemon has a src directory", file=sys.stderr)
        return 2

    # Only a carried component that IS in this tree can be stale. One that is
    # absent is a different tree, not a resolved entry - and a check that cannot
    # be run against a fixture is one whose own control cannot exist.
    present = {n for n in CARRIED if (REPO / n / "src").is_dir()}
    stale = sorted(present - carried_seen)
    if stale:
        print("Carried entries that no longer do both - delete them:\n")
        for name in stale:
            print(f"  - {name}")
        print("\nA carried allowance that outlives what it allowed is a hole a new one hides in.")
        return 1

    if findings:
        print("Daemons that fence themselves AND resolve a peer identity:\n")
        for name, why in findings:
            print(f"  - {name}: {why}")
        print(
            "\nThe fence denies `/proc/<pid>/exe` for a process outside its domain, so this"
            "\ndaemon refuses every caller and the refusal reads like policy. Drop the fence,"
            "\nor need less of the peer - SO_PEERCRED for the uid, a pidfd for liveness."
        )
        return 1

    print(
        f"{checked} daemon(s) checked, none both fences and resolves a peer "
        f"({len(CARRIED)} carried)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
