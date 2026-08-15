#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""No user unit relies on `IPAddress*`, which the user manager cannot apply.

WHY THIS EXISTS. systemd's IP firewalling is implemented by attaching a BPF
program to the unit's cgroup, and that needs privileges the per-user manager does
not have. A user unit carrying `IPAddressDeny=any` is not rejected and does not
fail - systemd logs one line, once per boot, for the FIRST unit that tries:

    arlen-store-backend.service: unit configures an IP firewall, but not
    running as root.
    (This warning is only shown for the first unit using IP firewalling.)

Note the second line. Only the first offender is named, so a second unit with the
same directive is silent, and the directive still reads in the unit file exactly
like a network restriction that is being enforced.

This was found on a boot, not by reading: `arlen-ai-proxy.service` carries
`IPAddressAllow=localhost` + `IPAddressDeny=any` - the containment for the one
component that is allowed to reach the network at all - and it has been inert for
as long as the unit has been per-user.

WHAT TO USE INSTEAD.

    no network at all      RestrictAddressFamilies=AF_UNIX (seccomp-based, so it
                           works in a user unit and actually blocks socket(2))
    host or route filtering  not expressible in a user unit. That is what the
                           egress enforcer in sdk/net-guard exists for: a netns
                           plus a forced proxy, applied by the launcher.

An entry in KNOWN records a unit that keeps the directive on purpose, with the
reason, so "we know it is inert" stays distinguishable from "nobody has looked".
"""

import sys
from pathlib import Path

OWN_TREE = len(sys.argv) <= 1
REPO = Path(__file__).resolve().parents[2] if OWN_TREE else Path(sys.argv[1]).resolve()

USER_UNITS = REPO / "dev/mkosi/mkosi.extra/usr/lib/systemd/user"

DIRECTIVES = ("IPAddressAllow=", "IPAddressDeny=", "IPIngressFilterPath=", "IPEgressFilterPath=")

# Units that keep an inert directive deliberately, and why. The reason has to say
# what actually enforces the restriction, not that the directive is harmless.
KNOWN = {
    "arlen-ai-proxy.service": (
        "localhost-only egress cannot be expressed in a user unit at all - "
        "RestrictAddressFamilies cannot tell a loopback connect from a remote "
        "one. The directive is kept as the statement of intent and the real "
        "enforcement is the netns-plus-proxy egress enforcer in sdk/net-guard, "
        "which is not yet applied to this unit. Documented in the unit itself."
    ),
}


def offenders(path: Path) -> list[tuple[int, str]]:
    out = []
    for n, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        s = line.strip()
        if s.startswith("#") or s.startswith(";"):
            continue
        if any(s.startswith(d) for d in DIRECTIVES):
            out.append((n, s))
    return out


def main() -> int:
    if not USER_UNITS.is_dir():
        print(f"NOTHING WAS READ: no user units under {USER_UNITS}", file=sys.stderr)
        return 2

    units = sorted(USER_UNITS.glob("*.service"))
    if not units:
        print(f"NOTHING WAS READ: no .service files in {USER_UNITS}", file=sys.stderr)
        return 2

    problems = []
    excused = 0
    for u in units:
        hits = offenders(u)
        if not hits:
            continue
        if u.name in KNOWN:
            excused += 1
            continue
        for n, text in hits:
            problems.append(f"{u.name}:{n}: {text}")

    if problems:
        print("user unit(s) relying on an IP firewall the user manager cannot apply:")
        for p in problems:
            print(f"  {p}")
        print(
            "\n  systemd attaches IP firewalling to the cgroup with BPF, which the\n"
            "  per-user manager has no privileges for. The unit starts, the\n"
            "  directive does nothing, and only the FIRST such unit per boot gets a\n"
            "  warning - so this reads as enforcement while enforcing nothing.\n\n"
            "  For 'no network at all' use RestrictAddressFamilies=AF_UNIX, which is\n"
            "  seccomp-based and works unprivileged. For host or route filtering use\n"
            "  the egress enforcer in sdk/net-guard. If the directive stays as a\n"
            "  statement of intent, add the unit to KNOWN with what actually\n"
            "  enforces it."
        )
        return 1

    print(
        f"OK: {len(units)} user unit(s), none relies on an unappliable IP firewall "
        f"({excused} excused with a recorded reason)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
