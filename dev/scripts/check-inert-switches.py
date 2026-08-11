#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A protection that is wired in code and never switched on in the image.

This shape has bitten four times now, and every time it took a night to find,
because the code reads correct and the fallback succeeds quietly:

    F3-2   `permission-helper.service` had no `ReadWritePaths` for the directory
           `record_identity` writes, so every RecordIdentity failed EROFS under
           `ProtectSystem=strict` and the built inode gate gated nothing.
    AL-1   the audit ingest ADMITTED list omitted `ai-agent`, so the agent's
           fail-closed gate-audit-before-act refused every action in release
           while debug masked it through the `dev.*` allowance.
    broker `ARLEN_CONFIG_BROKER_IDENTITY_UID` is set nowhere, so in a release
           build `broker_expected_uid` is None and the stamped-identity Tier 1
           refuses without connecting - the launcher stamp cannot be believed.
    bus    `ARLEN_EVENT_BUS_ENFORCE` is a deliberate shadow default, which is
           fine, but nothing recorded that the observation it waits on had not
           been done.

The inventory below is the standing answer to "is this switch on, and if not,
why not". It is hand-kept ON PURPOSE. Auto-classifying an env as a protection
rather than a test override is the same mistake as a check built from the
instances it happened to find: `ARLEN_PERMISSIONS_DIR` and
`ARLEN_AUDIT_EXTRA_ADMIT` are also unset in the image and MUST stay that way,
and no pattern separates them from the ones above. A person decides; this file
keeps the decision from going stale.

Two ways an entry rots, and the check is exactly those two:

    the env stops existing   a rename or deletion leaves the reason pointing at
                             nothing, and an inventory of absent switches reads
                             reassuring while saying nothing.
    the image state moves    someone sets or unsets it without touching the
                             reason, so the recorded justification now argues
                             for a state the tree is not in.

What it deliberately does NOT do is have an opinion about which state is right.
`ARLEN_EVENT_BUS_ENFORCE=1` is a rollout decision with real breakage behind it;
this check would fail on the flip only until the reason is updated to match,
which is the point - the flip and its written justification land together.
"""

import re
import subprocess
import sys
from pathlib import Path

REPO = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

IMAGE = "dev/mkosi/mkosi.extra"

# env -> (expected image state, what turning it on buys, why it is in that state)
SWITCHES = {
    "ARLEN_OWNER_USER": (
        "set",
        "the knowledge daemon serves a cross-uid first-party peer only when it is "
        "the named desktop owner, so another human user on the box cannot reach "
        "this user's graph through a canonical binary",
        "set to `arlen` in the arlen-graph system drop-in. A name, not a number: "
        "the socket sits at the shared /run/arlen and the desktop uid is whatever "
        "useradd picked, so a hardcoded 1000 would refuse the real user and leave "
        "the graph unreachable - failing in the direction nobody notices.",
    ),
    "ARLEN_CONFIG_BROKER_IDENTITY_UID": (
        "unset",
        "the stamped-identity Tier 1 believing the config-broker, which is the "
        "only peer resolution that survives a mount-namespace sandbox",
        "UNSET IS A GAP, not a default. A release build with no env makes "
        "`broker_expected_uid` None and the lookup refuses without connecting, so "
        "Tier 1 falls through to /proc on the booted image. The value today would "
        "be 0, because the image packages the broker as User=root (see the KNOWN "
        "DRIFT in check-packaged-units.sh); provisioning the arlen-config user "
        "must change this in the same commit. Setting it changes nothing "
        "observable until `[launcher] confined` is on, which is human-gated, so "
        "it is recorded rather than scaffolded ahead of the flip.",
    ),
    "ARLEN_EVENT_BUS_ENFORCE": (
        "unset",
        "the bus REJECTING an out-of-scope publish or subscribe instead of only "
        "logging it",
        "deliberate shadow default, the same cutover shape as stamped identity: "
        "the first-party [event_bus] scopes are meant to be verified against real "
        "traffic before the reject flip, because enforcing against profiles that "
        "do not declare their scopes silently stops delivery. The observation is "
        "the precondition and it has not been done on a booted image.",
    ),
    "ARLEN_CAPSULE_REQUIRE_FENCE": (
        "unset",
        "capsuled refusing to start rather than running unconfined when the "
        "Landlock write-fence cannot be enforced",
        "deliberate: a kernel that cannot enforce the fence leaves the daemon "
        "exactly as safe as no fence, so degrading is not a loss against the "
        "pre-fence baseline. Turning it on is a hardened-deployment choice that "
        "needs to know the image's kernel enforces Landlock, which is a boot "
        "question and not a tree question.",
    ),
}


def is_read_in_tree(env):
    """Does any Rust source still read this env?"""
    r = subprocess.run(
        ["git", "grep", "-l", "--", env, "--", "*.rs"],
        cwd=REPO,
        capture_output=True,
        text=True,
    )
    return bool(r.stdout.strip())


def image_state(env):
    """`set` if any file the image ships mentions the env, else `unset`."""
    root = REPO / IMAGE
    if not root.is_dir():
        return None
    pattern = re.compile(rf"\b{re.escape(env)}\s*=")
    for p in root.rglob("*"):
        if not p.is_file():
            continue
        try:
            if pattern.search(p.read_text(encoding="utf-8", errors="replace")):
                return "set"
        except OSError:
            continue
    return "unset"


def main():
    if not (REPO / IMAGE).is_dir():
        print(f"inert switches: no {IMAGE}; nothing to check")
        return 0

    problems = []
    for env, (expected, buys, why) in sorted(SWITCHES.items()):
        if not is_read_in_tree(env):
            problems.append(
                f"{env}: inventoried here, read by no Rust source. Either it was "
                f"renamed and this entry now argues about nothing, or the "
                f"protection was removed and the entry should go with it."
            )
            continue
        actual = image_state(env)
        if actual != expected:
            problems.append(
                f"{env}: recorded as {expected} in the image, actually {actual}.\n"
                f"    it controls: {buys}\n"
                f"    the recorded reason: {why}\n"
                f"    Update the reason in this file in the same commit that "
                f"moved the switch, so the justification describes the tree."
            )

    if problems:
        print("inert switches: a protection switch no longer matches its reason")
        for p in problems:
            print(f"  {p}")
        return 1

    off = [e for e, (x, _, _) in SWITCHES.items() if x == "unset"]
    print(f"OK: {len(SWITCHES)} protection switch(es) match their recorded state")
    if off:
        print(f"  OFF in the image ({len(off)}), each with a reason above: " + ", ".join(sorted(off)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
