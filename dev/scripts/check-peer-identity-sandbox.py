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

**`ARLEN_STAMPED_IDENTITY=enforce` does NOT lift this.** An earlier version of
this file split the units by that flag and told anyone reading that the enforced
ones could have their sandboxes back, because they resolve callers through a
pidfd. That was wrong and is retracted. `ConnectionAuth::extract_from` runs
`let legacy_app_id = app_id_from_pid(peer_pid)?` unconditionally and BEFORE it
looks at the mode (`connection_auth.rs:122`), so the `/proc/<pid>/exe` read
happens either way and its failure propagates; the enforce arm then calls
`pid_start_time(spid)?`, which is another `/proc` read. Hardening a unit on
enforce would break it exactly as the unhardened comment in each unit says.

The flag is the identity story getting stronger. It is not yet the /proc
dependency going away, and those are easy to confuse - I confused them for four
hours. What removes the dependency is the legacy resolution stopping, which is
the keystone work, not a unit-file change.

Narrowed by measurement on 11 Aug, because "reads /proc" does not say which read:

    ProtectSystem=strict     /proc/<pid>/stat = OK     /proc/<pid>/exe = DENIED

**Only the exe magic link is refused** - it is ptrace-gated, `stat` is not, so
`pid_start_time` survives a sandbox and is not the obstacle. For a unit already
on enforce the whole problem is therefore the single fatal `app_id_from_pid`
call, whose value enforce consumes only for the divergence log. Making a
non-authoritative observation unable to fail the connection is the first slice,
and it wants a boot against the audit chain rather than an argument.
"""

import re
import sys
from pathlib import Path

REPO = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

UNIT_DIR = "dev/mkosi/mkosi.extra/usr/lib/systemd/user"

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
    units = sorted((REPO / UNIT_DIR).glob("*.service")) if (REPO / UNIT_DIR).is_dir() else []
    flagged, carried, checked = [], [], 0

    unhardened: list[str] = []

    for unit in units:
        text = unit.read_text(encoding="utf-8")
        directives = [d for d in MOUNT_NS if re.search(rf"^{d}=", text, re.M)]
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
    if unhardened:
        print(f"PAYING THE OTHER HALF ({len(unhardened)}): these resolve peers and carry no "
              "mount-namespace hardening,")
        print("  which is what it costs to keep /proc identity working: "
              + ", ".join(sorted(unhardened)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
