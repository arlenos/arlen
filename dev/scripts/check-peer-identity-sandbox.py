#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A daemon cannot both live in its own mount namespace and identify its peers.

`app_id_from_pid` reads `/proc/<peer-pid>/exe`. That read is refused when the
READER runs in its own mount namespace, which is what most systemd hardening
gives a unit.

This was host-proved on 14 July and is written down in
`docs/architecture/stamped-identity-plan.md` (the bullet beginning "Brittle +
forces a bad hardening tradeoff"), including the root-bypass via CAP_SYS_PTRACE
and the user-namespace hypothesis it refuted. It was re-derived from scratch on
10 August, over five image builds, by someone who had read the `landlock-fence`
note and not the plan for the subject - so the rule was known and nothing
enforced it, which is what this file is for. Re-measured then with
`systemd-run --user`, one directive per run, against a process plainly readable
from outside the sandbox:

    PrivateDevices=yes        DENIED      RestrictNamespaces=yes  readable
    ProtectKernelTunables=yes DENIED
    ProtectSystem=strict      DENIED
    ProtectHome=read-only     DENIED
    PrivateTmp=yes            DENIED
    ProtectControlGroups=yes  DENIED

Every directive that creates a mount namespace denies it; the one that does not
create one leaves it working. No `/proc`-facing knob restores it - `MountAPIVFS=no`,
`ProtectProc=default` and `ProcSubset=all` were all tried and all stayed denied.
So it is hardening or exe-based peer identity, and a unit that takes both gets a
daemon that refuses or misidentifies every caller.

That is not theoretical. `arlen-ai-undo-signer` ships with six of those directives
and turns away every caller on the booted image, every boot, which means the undo
path - the thing that reverses what the agent did - does not work. It took most of
a night to find because the symptom is one `Permission denied` in a boot log and
four different mechanisms fit it plausibly.

The rule, deliberately narrow:

    a shipped user unit with a mount-namespace directive whose crate references
    `app_id_from_pid` or `ConnectionAuth` is flagged, unless it is listed below
    with a reason.

The exception list is for cases that are known and being carried, and it must
carry the reason rather than the name alone - a bare name is how an exception
outlives the problem it was granted for.

`ARLEN_STAMPED_IDENTITY=enforce` did not use to lift this, and the history is
worth keeping because the mistake was subtle. An earlier version of this file
split the units by that flag and said the enforced ones could have their
sandboxes back, since they resolve callers through a pidfd. That was wrong:
`ConnectionAuth::extract_from` ran `let legacy_app_id = app_id_from_pid(peer_pid)?`
unconditionally and BEFORE it looked at the mode, so the `/proc/<pid>/exe` read
happened either way and its failure propagated. The flag was the identity story
getting stronger, which is not the same thing as the /proc dependency going away,
and the two are easy to confuse - I confused them for four hours.

Narrowed by measurement on 11 Aug, because "reads /proc" does not say which read:

    ProtectSystem=strict     /proc/<pid>/stat = OK     /proc/<pid>/exe = DENIED

Only the exe magic link is refused - it is ptrace-gated, `stat` is not, so
`pid_start_time` survives a sandbox and was never the obstacle. I then said the
single fatal `app_id_from_pid` call was therefore the whole problem, and
`resolve_identity` in `connection_auth.rs` removed it: the legacy value is passed
as a `Result`, the shadow arm unwraps it because it has nothing else to be, the
enforce arm drops it.

**That was necessary and not sufficient, and this is the measurement rather than
the argument.** A probe daemon under `systemd-run --user -p ProtectSystem=strict`
with `ARLEN_STAMPED_IDENTITY=enforce`, given a same-uid peer:

    stat = OK    exe = DENIED    extract_from = REFUSED (CannotReadExe)

`stamped_identity::app_id_from_connection_at` resolves in tiers. Tier 1 asks the
config-broker for the launcher stamp and returns with no `/proc` read at all;
Tiers 2 and 3 fall through to `exe_path_openat(peer.pid())`, which the sandbox
refuses exactly like the legacy call did. The probe's peer was a plain binary and
had never been through `arlen-run`, so it fell through and was refused.

So the condition for hardening a unit is not "the unit is on enforce". It is
"every caller of that unit is launcher-stamped", which is a claim about the
callers and about the config-broker being reachable, not about the unit file.

Tier 1 itself was then measured, same sandbox, with a real `arlen-config-broker`
and a peer registered through `register_identity`:

    ProtectSystem=strict, enforce, stamped peer      exe = DENIED   extract = OK
    the same, with the legacy `?` put back           exe = DENIED   extract = REFUSED

Both halves matter. The first says a hardened enforced daemon CAN identify a
stamped caller with `/proc/<pid>/exe` refused to it. The second is the A/B that
makes removing the legacy read load-bearing rather than tidy: with that one line
back, the identical setup fails before Tier 1 is ever consulted.

**And here is why this check still flags a hardened enforced unit anyway.** Tier 1
only knows a process that `arlen-run` stamped at spawn. No shipped unit is spawned
that way - 0 of 17 have `arlen-run` in their `ExecStart`, and `broker_lookup`'s own
comment says system daemons resolve via `/proc`, never the broker. So the tier that
survives a sandbox covers APPS, and a socket whose callers are other daemons has no
Tier 1 to fall into. `arlen-auditd` is exactly that: its ingest producers are
daemons, so hardening it would still refuse them.

Relax this rule per socket, never globally, and only for one whose callers are
launcher-stamped in the booted image - shown by a boot, not by a unit test. Three
times now the reasoning was clean and the measurement disagreed.
"""

import re
import sys
from pathlib import Path

REPO = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

# BOTH shipped unit trees. It read only the user tree for its first weeks, which
# is how it missed the loudest instance of its own subject: `arlen-event-bus` is a
# SYSTEM unit carrying five mount-namespace directives while resolving every peer
# through `/proc/<pid>/exe`, and this check walked straight past it. A rule that
# looks in one of two places is not a narrower rule, it is a rule with a hole.
UNIT_DIRS = (
    "dev/mkosi/mkosi.extra/usr/lib/systemd/user",
    "dev/mkosi/mkosi.extra/usr/lib/systemd/system",
)

# Directives that put the unit in its own mount namespace. `RestrictNamespaces`
# is deliberately absent: it restricts what the unit may CREATE and does not put
# it in one, and it was the negative control that made the rule measurable.
MOUNT_NS = (
    "ProtectSystem",
    "ProtectHome",
    "PrivateTmp",
    "PrivateDevices",
    "ProtectKernelTunables",
    "ProtectControlGroups",
    "PrivateMounts",
    "ReadWritePaths",
)

RESOLVES_PEER = ("app_id_from_pid", "ConnectionAuth")

# `/proc/<pid>/exe` is ptrace-gated, and CAP_SYS_PTRACE is the documented bypass -
# the same one `stamped-identity-plan.md` records from the 14 July host-proof. A
# unit with no `User=` runs as root and keeps the default bounding set, so it reads
# the link a same-uid reader is refused, and the whole defect does not apply to it.
#
# This matters because it is what widening to the system tree first got wrong:
# `arlen-event-bus` and `arlen-graph` are both hardened AND both resolve peers, and
# flagging them looked for a moment like two live findings. They run as root. Two
# false positives in a gate are worse than the hole that hid them, because they
# teach people to skip the output.
#
# NB inherited rather than re-measured: my own measurement covered a same-uid
# non-root reader. If a unit ever gains `CapabilityBoundingSet=` without
# CAP_SYS_PTRACE, or a `User=`, it drops back into the rule and this check says so.
def runs_as_root_with_ptrace(text, system_unit):
    # SYSTEM units only. A user unit has no `User=` either, because it already runs
    # as the user - and reading absence as root there exempts exactly the wrong
    # thing: the first version of this rule waved through `arlen-ai-undo-signer`,
    # the one CONFIRMED live breakage this whole file was written for. An exemption
    # that swallows the motivating case is worse than no exemption.
    if not system_unit:
        return False
    if re.search(r"^User=", text, re.M):
        return False
    bounding = re.search(r"^CapabilityBoundingSet=(.*)$", text, re.M)
    if bounding and "CAP_SYS_PTRACE" not in bounding.group(1):
        return False
    return True

# name -> why it is carried. Empty is the state to aim for.
KNOWN = {
    "arlen-ai-undo-signer": (
        "the live instance of this defect, found 10 Aug: it refuses every caller "
        "on the image. Dropping the hardening is not the fix (it holds the undo "
        "log's HMAC key); the identity step has to stop depending on the /proc "
        "read, which is the stamped-identity work. Delete this entry when it does."
    ),
    "arlen-notifyd": (
        "the same defect with the other failure mode, and this check is what found "
        "it - a hand audit of the same question had missed it. `server.rs:205` calls "
        "app_id_from_pid(...).ok(), so an unresolvable peer becomes None rather than "
        "a refusal: notifications keep working and quietly lose which app sent them. "
        "Same fix, same entry to delete."
    ),
}


def crate_for(binary):
    """The crate directory whose package or bin is named `binary`."""
    for toml in list(REPO.glob("daemons/*/Cargo.toml")) + list(REPO.glob("ai/*/Cargo.toml")):
        text = toml.read_text(encoding="utf-8")
        if re.search(rf'^name = "{re.escape(binary)}"', text, re.M):
            return toml.parent
    return None


def resolves_peers(crate):
    for rs in crate.rglob("*.rs"):
        if "/target/" in str(rs):
            continue
        try:
            text = rs.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        if any(n in text for n in RESOLVES_PEER):
            return rs.relative_to(REPO)
    return None


def main():
    units = sorted(
        u
        for d in UNIT_DIRS
        if (REPO / d).is_dir()
        for u in (REPO / d).glob("*.service")
    )
    flagged, carried, checked = [], [], 0
    root_exempt: list[str] = []

    unhardened: list[str] = []

    for unit in units:
        text = unit.read_text(encoding="utf-8")
        is_system = unit.parent.name == "system"
        directives = [d for d in MOUNT_NS if re.search(rf"^{d}=", text, re.M)]
        if directives and runs_as_root_with_ptrace(text, is_system):
            # Sandboxed, resolves peers, and reads the link anyway. Counted so the
            # summary cannot read as "nothing here is sandboxed".
            root_exempt.append(unit.stem)
            continue
        if not directives:
            # The other side of the same trade, and the reason a small flagged list
            # is not good news: a daemon that resolves peers and carries NO
            # mount-namespace hardening has most likely given the hardening up to
            # keep identity working. `stamped-identity-plan.md` records exactly that
            # as the interim fix for the consent broker, and notes those daemons are
            # "running WEAKER for it". Counting only the broken-and-hardened hides
            # that cost completely, so list these too - not as failures, since the
            # trade was deliberate, but so it is visible what it is costing.
            exec_line = re.search(r"^ExecStart=(\S+)", text, re.M)
            if exec_line:
                crate = crate_for(Path(exec_line.group(1)).name)
                if crate is not None and resolves_peers(crate) is not None:
                    unhardened.append(unit.stem)
            continue
        exec_line = re.search(r"^ExecStart=(\S+)", text, re.M)
        if not exec_line:
            continue
        crate = crate_for(Path(exec_line.group(1)).name)
        if crate is None:
            continue
        checked += 1
        where = resolves_peers(crate)
        if where is None:
            continue
        name = unit.stem
        if name in KNOWN:
            carried.append(f"{name}: {KNOWN[name]}")
        else:
            flagged.append(
                f"{name}: {', '.join(directives)} put it in its own mount namespace, "
                f"and {where} identifies its peer by reading /proc/<pid>/exe, which "
                f"that namespace refuses. It will misidentify or refuse every caller."
            )

    if flagged:
        print("peer identity: a hardened unit cannot read its callers' identity")
        for f in flagged:
            print(f"  {f}")
        print("  Either the unit keeps its sandbox and the daemon stops resolving")
        print("  identity through /proc, or - if this is known and being carried -")
        print("  add it to KNOWN with the reason.")
        return 1

    print(f"OK: {checked} hardened unit(s) checked, none newly unable to identify callers")
    for c in carried:
        print(f"KNOWN (not failing): {c}")
    if root_exempt:
        print(f"ROOT, SO EXEMPT ({len(root_exempt)}): sandboxed and peer-resolving, but running as root "
              "with CAP_SYS_PTRACE,")
        print("  which reads the exe link a same-uid reader is refused: "
              + ", ".join(sorted(root_exempt)))
    if unhardened:
        print(f"PAYING THE OTHER HALF ({len(unhardened)}): these resolve peers and carry no "
              "mount-namespace hardening,")
        print("  which is what it costs to keep /proc identity working: "
              + ", ".join(sorted(unhardened)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
