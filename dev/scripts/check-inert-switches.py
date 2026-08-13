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
    broker `ARLEN_CONFIG_BROKER_IDENTITY_UID` was set nowhere, so in a release
           build `broker_expected_uid` was None and the stamped-identity Tier 1
           refused without connecting - the launcher stamp could not be believed.
           Closed 13 Aug; the entry below records what the boot measured.
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
    "ARLEN_STAMPED_IDENTITY": (
        "set",
        "peer identity taken from the pinned pidfd (app_id and pid both) rather "
        "than a /proc/{pid}/exe read the resolver only re-checks afterwards",
        "PARTIAL, and recorded as `set` because that is all this check can see. "
        "It is a PER-PROCESS switch - `ConnectionAuth` reads it in whichever "
        "process is resolving - and exactly two units carry `=enforce` today, "
        "auditd and the consent broker. Every other reader is still on the "
        "shadow default - which was recorded here as keeping the legacy app_id and "
        "only logging where the two disagree, and that is FALSE for exactly the "
        "readers this matters to. Measured 12 Aug on the booted image: the "
        "undo-signer rejected all three of the AI engine daemon's connections with "
        "`cannot read exe path for pid 647: Permission denied`, and its undo.log is "
        "0 bytes. Shadow mode falls back to the legacy /proc resolver, and for a "
        "reader under ProtectSystem=strict that resolver CANNOT RUN, so there is "
        "nothing to fall back to and shadow is indistinguishable from enforce. The "
        "signer is otherwise entirely correct - key custody, socket, permissions - "
        "and seals nothing, which is the AL-1 shape again: wired in code, broken in "
        "deployment. NB the stamp is minted by arlen-run, which launches APPS, so "
        "even flipping `confined` leaves daemon-to-daemon peers like this one with "
        "no stamp at all; the keystone has to answer that case too. Worth tracking despite the coarse state, because this is "
        "the switch the hardening strand turns on: a reader under "
        "ProtectSystem=strict is refused /proc/{pid}/exe entirely, so for those "
        "the stamped path is not a stronger option but the only working one. "
        "Rolling it further means reading the divergence the shadow mode is "
        "already logging, not flipping on faith.",
    ),
    "ARLEN_CONFIG_BROKER_IDENTITY_UID": (
        "set",
        "the stamped-identity Tier 1 believing the config-broker, which is the "
        "only peer resolution that survives a mount-namespace sandbox",
        "SET ON 13 Aug, after a boot showed what unset actually cost. It was "
        "recorded as harmless until `[launcher] confined` flips, and that was "
        "wrong: the same env gates every peer resolution, so with it unset the "
        "undo signer rejected four connections on every boot with "
        "`readlinkat(exe): Permission denied` - the tier was not merely dormant, "
        "it was failing live. Setting it took those to zero. NOT set as a literal "
        "in any unit: sysusers allocates the uid, and the image packages the "
        "broker as User=root against a dist unit that says arlen-config (the KNOWN "
        "DRIFT in check-packaged-units.sh), so a name-derived value would be "
        "wrong in exactly the deployment that ships. It is derived from the owner "
        "of the SYSTEM socket, which is true under either user and is trustworthy "
        "because /run/arlen is not user-writable - by the session for what it "
        "starts, and by the user-environment generator for user services that "
        "start before the session's import.",
    ),
    "ARLEN_EVENT_BUS_ENFORCE": (
        "set",
        "the bus REJECTING an out-of-scope publish or subscribe instead of only "
        "logging it",
        "FLIPPED ON 12 Aug, in the release image, after the condition D3 named was "
        "met: populate-then-flip fail-closed - flip once the declarations are "
        "populated and a boot shows no denials. The boot showed zero. Before that "
        "this entry was wrong TWICE in the same direction, both times by reasoning "
        "about the flip instead of reading what shadow mode had logged; the second "
        "time a boot journal refuted it outright, with the compositor publishing as "
        "`app_id=\"<unresolved>\"` because it was installed under the upstream "
        "fork's name. That is fixed (it is `/usr/bin/arlen-compositor` now and "
        "declares its publish list) and the flip was then measured, not argued. "
        "WHAT THE GREEN COVERS is written in full beside the switch itself, in "
        "`arlen-event-bus.service.d/10-enforce.conf`, because \"zero denials\" is "
        "true of what ran rather than of what ships: three declared components are "
        "exercised by a boot, `dev.arlen.files` is covered by "
        "`check-bus-declarations.py` reading its profile instead, everything else "
        "declares nothing and is exempt by tier, and an undeclared non-system "
        "caller is the case the flip exists to bite.",
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
    # The image tree is committed source, not build output, so its absence is not
    # "no image has been built yet" - it is a scan pointed somewhere else. This
    # gate's whole subject is a protection that is recorded as on and is not, and
    # exiting 0 having read no image is the same shape one level up.
    if not (REPO / IMAGE).is_dir():
        print(f"NOTHING WAS READ: no {IMAGE} under {REPO}", file=sys.stderr)
        return 2

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
