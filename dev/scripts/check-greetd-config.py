#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Check that everything `/etc/greetd/config.toml` names exists on the image.

WHY THIS EXISTS. greetd's config is the first thing that runs on a graphical boot
and the last thing anybody reads. It names three kinds of thing by string - a
command, a user, a PAM service - and every one of them fails at the worst possible
moment if it is absent: not at build time, not in a test, but on a machine showing
a black screen with no session to log in from and no way to read the journal.

Each was reachable when this was written, on 28 August:

- **The command.** `[default_session]` ran `/usr/bin/arlen-session`, which the image
  really does install. Had it named the greeter - which was the whole point of the
  app sitting finished in `apps/greeter` - nothing would have noticed that no build
  step staged it.
- **The user.** `_greetd` comes from Debian's greetd package, `arlen` from
  `mkosi.postinst`. A session naming any other user starts as nobody at all.
- **The PAM service.** greetd defaults `[initial_session]` and a typed-password
  login to the SAME service, `greetd`, and this image's copy of that file permitted
  every password. Splitting them - `greetd-autologin` for the boot, `greetd` for a
  real login - is what let one be strict, and it introduces a new way to fail: a
  service naming a file that does not ship falls through to `/etc/pam.d/other`,
  which on Debian denies. So the config would look right and nobody would get in.

The command is checked against both halves of how a binary reaches the image: a
build step that installs it, or a Debian package named in `mkosi.conf` whose name
is the binary's. That second rule is deliberately literal - `cage` provides
`/usr/bin/cage` - and it will not recognise a package that ships a differently
named binary. When that comes up, the fix is to name it in PROVIDED_BY_PACKAGE with
the package it comes from, which is a sentence a reader can check.
"""

from __future__ import annotations

import re
import sys
import tomllib
from pathlib import Path

ROOT = (
    Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
)
MKOSI = ROOT / "dev/mkosi"
CONFIG = MKOSI / "mkosi.extra/etc/greetd/config.toml"
STEPS = MKOSI / "mkosi.build.d"
PAMD = MKOSI / "mkosi.extra/etc/pam.d"
POSTINST = MKOSI / "mkosi.postinst"
PACKAGES = MKOSI / "mkosi.conf"

# A binary whose path does not match its package name, and where it comes from.
# Empty today: `cage` is `/usr/bin/cage`. An entry here is a claim somebody can
# check against the package, which is the point of writing it down rather than
# widening the rule.
PROVIDED_BY_PACKAGE: dict[str, str] = {}

# Users the image creates outside its own postinst, and by what. greetd's Debian
# package creates `_greetd` (`GREETDUSERGROUP=_greetd`, home /var/lib/greetd), which
# is also what `apps/greeter/dist/arlen-greeter.tmpfiles.conf` names.
USERS_FROM_PACKAGES = {"_greetd": "the greetd package"}


def packages() -> set[str]:
    """The Debian packages `mkosi.conf` asks for, read out of its `Packages=` block."""
    if not PACKAGES.is_file():
        return set()
    out: set[str] = set()
    in_block = False
    for line in PACKAGES.read_text(encoding="utf-8").splitlines():
        if line.startswith("Packages="):
            in_block = True
            continue
        if in_block:
            # The block ends at the next unindented line: a new key or a new section.
            if line and not line[0].isspace():
                break
            name = line.strip()
            if name and not name.startswith("#"):
                out.add(name)
    return out


def installed_paths() -> dict[str, str]:
    """Absolute image path to the build step that installs it."""
    out: dict[str, str] = {}
    pattern = re.compile(r'\$DESTDIR(/[A-Za-z0-9._/${}-]+)')
    for step in sorted(STEPS.glob("*.chroot")):
        for m in pattern.finditer(step.read_text(encoding="utf-8", errors="replace")):
            out[m.group(1)] = step.name
    return out


def created_users() -> set[str]:
    """Users `mkosi.postinst` creates, read off its useradd lines."""
    if not POSTINST.is_file():
        return set()
    text = POSTINST.read_text(encoding="utf-8")
    return set(re.findall(r"useradd[^\n]*?\s([A-Za-z_][A-Za-z0-9_-]*)\s*$", text, re.MULTILINE))


def commands(session: dict) -> list[str]:
    """The absolute paths a session command runs.

    greetd hands the whole string to `sh -c`, so a command may be several binaries
    (`cage -s -- arlen-greeter` is two). Every absolute-path token is one of them;
    a bare word is left alone, since resolving it would mean guessing at PATH.
    """
    return [tok for tok in str(session.get("command", "")).split() if tok.startswith("/")]


def main() -> int:
    if not CONFIG.is_file():
        print(f"{CONFIG.relative_to(ROOT)} is missing; the image has no greetd config")
        return 1
    try:
        config = tomllib.loads(CONFIG.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as e:
        print(f"{CONFIG.relative_to(ROOT)} does not parse as TOML: {e}")
        return 1

    sessions = {k: v for k, v in config.items() if k.endswith("_session") and isinstance(v, dict)}
    if not sessions:
        print("greetd config declares no session; greetd refuses to start without one")
        return 1

    pkgs = packages()
    paths = installed_paths()
    users = created_users() | set(USERS_FROM_PACKAGES)
    problems: list[str] = []
    checked = 0

    for name, session in sorted(sessions.items()):
        cmds = commands(session)
        if not cmds:
            problems.append(
                f"[{name}] names no absolute command, so nothing here can confirm it exists"
            )
        for path in cmds:
            checked += 1
            basename = path.rsplit("/", 1)[-1]
            if path in paths:
                continue
            if basename in pkgs or PROVIDED_BY_PACKAGE.get(basename) in pkgs:
                continue
            problems.append(
                f"[{name}] runs {path}, which no build step installs and no package in "
                f"mkosi.conf is named for. greetd will exec it and get 'No such file or "
                f"directory' on a screen with nothing else on it. Stage it, add the "
                f"package, or name it in PROVIDED_BY_PACKAGE with where it comes from."
            )

        user = session.get("user")
        if user is None:
            problems.append(f"[{name}] names no user")
        elif user not in users:
            problems.append(
                f"[{name}] runs as '{user}', which mkosi.postinst does not create and no "
                f"entry in USERS_FROM_PACKAGES accounts for. greetd cannot resolve it and "
                f"the session never starts."
            )

        # greetd's own defaults, read off `config/mod.rs`: GREETER_SERVICE for the
        # default session (it is the greeter), GENERAL_SERVICE for everything else.
        # Checking only an explicit `service =` would miss the common case, which is
        # a config that names none and depends on a file it never mentions.
        service = session.get("service") or (
            "greetd-greeter" if name == "default_session" else "greetd"
        )
        if not (PAMD / str(service)).is_file():
            problems.append(
                f"[{name}] names PAM service '{service}' and "
                f"{(PAMD / str(service)).relative_to(ROOT)} does not ship. PAM falls back to "
                f"/etc/pam.d/other, which denies, so every login through it fails."
            )

    if problems:
        for p in problems:
            print(p)
        return 1

    print(
        f"greetd config: {len(sessions)} session(s), {checked} command path(s), each a "
        f"binary the image installs, run as a user it creates, under a PAM file it ships"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
