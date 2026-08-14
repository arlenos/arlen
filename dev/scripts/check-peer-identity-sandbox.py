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
    "arlen-graph": (
        "ARRIVED 15 Aug WITH THE PER-USER MOVE, and it is the same defect as the "
        "undo signer's below rather than a new one. As a SYSTEM unit this daemon "
        "read its peers' /proc fine - not because its sandbox was weaker, but "
        "because it ran as root, and the refusal this check describes falls on "
        "non-root readers. Measured on the 14 Aug boot: the event bus (system, "
        "root) resolved every peer it saw while the undo signer (user, uid 1000) "
        "refused every one, on identical targets.\n"
        "    So moving it under the user manager, which the product needs, costs "
        "it the root exemption it was silently relying on. That is worth stating "
        "plainly: the per-user topology and /proc-based peer identity cannot both "
        "be had, and the second is the one that has to go.\n"
        "    Carried rather than fixed here because the fix is the same one the "
        "signer waits on - a launcher stamp producer - and inventing a second "
        "route for this one daemon would leave two identity paths to reason "
        "about. What it costs meanwhile: the graph's read-scope gate cannot name "
        "its callers, so it falls back to whatever an unresolved peer gets."
    ),
    "arlen-ai-undo-signer": (
        "the live instance of this defect, found 10 Aug: it refuses every caller "
        "on the image. Dropping the hardening is not the fix (it holds the undo "
        "log's HMAC key); the identity step has to stop depending on the /proc "
        "read, which is the stamped-identity work.\n"
        "    That work is BUILT and still does not fix this, checked 11 Aug and "
        "re-checked 12 Aug: the stamped resolver has all three tiers, but only "
        "Tier 1 avoids /proc, and Tier 1 needs a launcher stamp that nothing "
        "produces here.\n"
        "    One premise of the 11 Aug reading has since changed and the "
        "conclusion has not. `arlen-run` IS installed now - "
        "`08r-arlen-run.sh.chroot` puts it in libexec with a `/usr/bin` symlink - "
        "so the blocker is no longer that the launcher is absent. What still "
        "blocks it is that nothing LAUNCHES through it: every desktop entry Execs "
        "its binary directly (`Exec=arlen-files`), and `[launcher] confined` "
        "defaults false with no shell.toml shipped to change it. So the two units "
        "that already set ARLEN_STAMPED_IDENTITY=enforce resolve through Tiers 2 "
        "and 3 every time, which is the same /proc read.\n"
        "    The blocker is still a producer rather than more resolver, but it is "
        "one step further along than this entry said, and someone acting on the "
        "old wording would add a build step that already exists.\n"
        "    13 Aug: the plan in this entry - stamped identity will fix it once a "
        "producer exists - is WRONG for a namespaced unit, and the boot said so. "
        "The stamped tier is now live on the image (the consent broker resolves "
        "stamps and logs divergence), and this daemon still refuses every caller, "
        "with a different error: `broker uid 65534 != expected 0`. Tier 1 avoids "
        "the /proc read, but the client still has to know it is talking to the "
        "real broker, and it authenticates that by uid - which cannot work from "
        "inside a namespace where the root broker has no number at all. So the "
        "namespace defeats BOTH routes, and no amount of producer fixes it.\n"
        "    14 Aug: NEITHER of those two ways out was taken, and the planner ruled "
        "for a third - keep the hardening, keep it a user unit, and stop "
        "authenticating the broker by peer uid on this connection. `/run/arlen` is "
        "root-owned mode 0755 (from `arlen-event-bus.service`'s "
        "`RuntimeDirectory=`), so a node inside it was bound by root and the "
        "connect is authenticated by the filesystem before any uid is consulted. "
        "That is not the rejected 'unmapped means the broker' inference: nothing is "
        "read off the peer at all. Built in `lookup_identity_authenticated`, with "
        "`check-runtime-dir-closed.py` guarding the directory mode the argument "
        "rests on.\n"
        "    So Tier 1 can now answer inside the namespace, and the "
        "session-supervisor registers every supervised unit, which is the producer "
        "the entry above said was missing. Both halves are in place ON PAPER.\n"
        "    Delete this entry when a BOOT shows the signer resolving a real "
        "caller, not before. Three times in this file the reasoning was clean and "
        "the measurement disagreed, and this is the fourth time the same chain has "
        "looked complete."
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
    # Units that resolve no peer, or are exempt, are legitimately not `checked`, so
    # that count cannot carry the guard. Finding no UNIT at all can only mean the
    # scan ran outside the tree: both directories are committed source.
    if not units:
        print(
            f"NOTHING WAS READ: no unit under {', '.join(UNIT_DIRS)} in {REPO}",
            file=sys.stderr,
        )
        return 2

    flagged, carried, checked = [], [], 0
    root_exempt: list[str] = []
    # KNOWN entries the loop actually reached. An entry it never reaches has
    # stopped describing a live defect - see the staleness check after the loop.
    reached: set[str] = set()

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
            reached.add(name)
        else:
            flagged.append(
                f"{name}: {', '.join(directives)} put it in its own mount namespace, "
                f"and {where} identifies its peer by reading /proc/<pid>/exe, which "
                f"that namespace refuses. It will misidentify or refuse every caller."
            )

    # An entry the loop never reached. Each one opens by calling itself a live
    # instance of this defect ("it refuses every caller on the image"), so the day
    # its unit stops qualifying - hardening dropped, peer resolution gone, unit
    # renamed or deleted - the entry is describing a machine that no longer exists
    # while reading as work somebody still owes. Nothing said so until 12 Aug,
    # which is also how `check-invoke-scope.py` came to carry two acknowledgements
    # of calls that had both been fixed.
    #
    # Deliberately structural: it can tell that an entry's SUBJECT no longer
    # qualifies, not that its PROSE has drifted. This entry's own reasoning had a
    # premise go stale (`arlen-run` is installed now) with the conclusion intact,
    # and no mechanical check could have caught that - only reading it again.
    #
    # Scoped to THIS tree, and that is not the hardcoded-root antipattern. The
    # check proper still runs against whatever tree it is handed; it is the
    # self-audit that cannot, because KNOWN is a set of claims about one specific
    # repo and a fixture tree lacks those units for reasons that have nothing to do
    # with the entries being stale. Without this, every clean fixture case turned
    # red - the same collision `check-spawned-binaries.py` hit an hour earlier,
    # where carrying stub subjects in the fixture WAS the right answer because a
    # stub spawn is one line. Here a stub would need a unit with mount-namespace
    # directives, a crate that resolves peers, and an entry in the binary-to-crate
    # mapping, which is enough fake tree to be its own source of error.
    audits_own_list = len(sys.argv) <= 1 or REPO == Path(__file__).resolve().parents[2]
    for name in sorted(set(KNOWN) - reached) if audits_own_list else []:
        flagged.append(
            f"{name} is carried as a known instance of this defect, but it no "
            f"longer qualifies - the unit is gone, or it dropped its "
            f"mount-namespace hardening, or its crate stopped resolving peers. "
            f"Re-read the entry and drop it if the defect is fixed."
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
