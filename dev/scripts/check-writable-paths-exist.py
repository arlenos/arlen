#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A unit may not name a writable path that nothing creates.

Under `ProtectSystem=strict`, systemd builds a mount namespace before the service
runs, and every `ReadWritePaths=` entry has to EXIST at that moment. One that does
not fails the whole unit at namespace setup - before its first line executes:

    permission-helper.service: Failed to set up mount namespacing:
      /var/lib/arlen/identity: No such file or directory
    permission-helper.service: Failed at step NAMESPACE

That is the real boot log from 19 Aug, the first image that carried the install
path. The line it died on had been ADDED as a fix: the helper writes the F3
identity registry, and without the path in `ReadWritePaths` every write failed
EROFS. The fix was right and it made the unit unbootable, because the directory
existed only on machines where the helper had already run once.

WHY A GATE AND NOT A NOTE. This cannot be caught by reading the unit, by building
the image, or by any test that runs where the developer works - the paths are all
present on a machine that has run the software before. It is visible only on a
FRESH boot, which is the one place nobody looks until something is broken. The
same shape has now bitten twice: this, and the knowledge daemon's FUSE mount under
`ProtectHome=read-only`.

WHAT COUNTS AS CREATED. A path is satisfied when a `tmpfiles.d` entry we ship
makes it, when an image build phase places a file inside it, when the unit declares `StateDirectory=`/`RuntimeDirectory=`/
`CacheDirectory=`/`LogsDirectory=` covering it, or when it lives under a directory
systemd itself guarantees - `/run/user/%U` exists for a logged-in user, `%t` is
that same runtime directory, and `%h` subpaths are the user's own to make. The
specifier forms are treated as prefixes rather than expanded, because the question
is whether SOMETHING creates the tree, not what it resolves to on this machine.

Run: dev/scripts/check-writable-paths-exist.py [repo-root]
"""

import pathlib
import re
import sys

ROOT = (
    pathlib.Path(sys.argv[1]).resolve()
    if len(sys.argv) > 1
    else pathlib.Path(__file__).resolve().parents[2]
)

#: Trees systemd or the session guarantee, so a path under one needs no creator.
#: `%t` and `/run/user/%U` are the runtime directory, which logind makes; `%h`
#: paths are inside the user's own home.
GUARANTEED = ("%t", "/run/user/%U", "%h", "/tmp", "/var/tmp")

DIRECTIVE = re.compile(r"^(\w+)=(.*)$")


#: Where OUR tmpfiles live. Globbing for `**/tmpfiles.d/*.conf` also finds
#: Debian's own, vendored under the mkosi cache and tools trees - which is how
#: the first version of this gate decided `/var/lib` was created (by apt's file)
#: and therefore that everything under it was fine.
OURS = ("dev/mkosi/mkosi.extra", "daemons", "apps", "distro")


def tmpfiles_paths(root: pathlib.Path) -> set[str]:
    """Every path a tmpfiles.d entry WE ship creates."""
    out: set[str] = set()
    for conf in root.glob("**/tmpfiles.d/*.conf"):
        if {"target", "mkosi.builddir", "node_modules", "mkosi.cache", "mkosi.tools"} & set(
            conf.parts
        ):
            continue
        rel = conf.relative_to(root).as_posix()
        if not rel.startswith(OURS):
            continue
        for line in conf.read_text().splitlines():
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            parts = line.split()
            # type path mode user group age argument
            if len(parts) >= 2 and parts[0].lstrip("+").startswith(("d", "D", "Z", "z", "f", "L")):
                out.add(parts[1])
    return out


def staged_dirs(root: pathlib.Path) -> set[str]:
    """Directories the image build populates, which therefore exist on it.

    `install -D` and `cp` create the parents of what they place, so a phase that
    stages an app into `/usr/lib/arlen/apps/<id>/bin/` is what makes
    `/usr/lib/arlen/apps` exist. Missing this source is why the first strict
    version flagged `install-helper.service`, whose two paths are real: the unit
    starts fine on the image, which the 19 Aug boot log shows.

    Every ancestor counts, not just the immediate parent, because creating a deep
    path creates the whole chain.
    """
    out: set[str] = set()
    steps = root / "dev/mkosi/mkosi.build.d"
    for phase in list(steps.glob("*")) + list((root / "dev/mkosi").glob("*.sh")):
        if not phase.is_file():
            continue
        for line in phase.read_text().splitlines():
            stripped = line.lstrip()
            if stripped.startswith("#"):
                continue
            if "install " not in stripped and not stripped.startswith("cp "):
                continue
            for token in re.findall(r'"?\$DESTDIR([^"\s]*)', stripped):
                node = pathlib.PurePosixPath(token)
                # The token may be a file or a directory; either way its parents
                # exist afterwards.
                for parent in list(node.parents):
                    if str(parent) != "/":
                        out.add(str(parent))
    # The base distribution's own directories. Named rather than inferred: these
    # are the ones our units write into, and pretending to enumerate Debian's
    # filesystem from here would be a guess dressed as a check.
    out.update({"/usr/share/applications", "/usr/share/dbus-1", "/etc"})
    return out


def units(root: pathlib.Path) -> list[pathlib.Path]:
    return [
        p
        for p in root.glob("**/dist/**/*.service")
        if not ({"target", "mkosi.builddir", "node_modules"} & set(p.parts))
    ]


def covered(path: str, creators: set[str]) -> bool:
    """Is `path` itself created?

    EXACT, not by prefix. Creating a parent does not create a child: a tmpfiles
    entry for `/var/lib` leaves `/var/lib/arlen/identity` just as absent, and the
    unit still dies at namespace setup. The first version of this check accepted
    an ancestor and so passed the very unit it was written for, which is the
    permissive direction and the one worth being strict about.
    """
    if path.startswith(GUARANTEED):
        return True
    return path.rstrip("/") in {c.rstrip("/") for c in creators}


def main() -> int:
    all_units = units(ROOT)
    if not all_units:
        print(f"NOTHING WAS READ: no unit files under {ROOT}", file=sys.stderr)
        return 2

    made_by_tmpfiles = tmpfiles_paths(ROOT) | staged_dirs(ROOT)
    findings = []
    checked = 0

    for unit in all_units:
        text = unit.read_text()
        strict = "ProtectSystem=strict" in text
        if not strict:
            continue
        writable: list[str] = []
        declared: set[str] = set()
        for line in text.splitlines():
            m = DIRECTIVE.match(line.strip())
            if not m:
                continue
            key, value = m.group(1), m.group(2)
            if key == "ReadWritePaths":
                writable.extend(v for v in value.split() if v and not v.startswith("-"))
            elif key in ("StateDirectory", "RuntimeDirectory", "CacheDirectory", "LogsDirectory"):
                base = {
                    "StateDirectory": "/var/lib/",
                    "RuntimeDirectory": "/run/",
                    "CacheDirectory": "/var/cache/",
                    "LogsDirectory": "/var/log/",
                }[key]
                declared.update(base + v for v in value.split() if v)
        if not writable:
            continue
        checked += 1
        for path in writable:
            if not covered(path, made_by_tmpfiles | declared):
                findings.append(
                    f"{unit.relative_to(ROOT)}: `ReadWritePaths={path}` and nothing "
                    f"creates it, so on a fresh boot the unit dies at namespace "
                    f"setup before it runs. Use `StateDirectory=` (systemd makes it "
                    f"first) or a tmpfiles.d entry."
                )

    if findings:
        print(
            f"{checked} strict unit(s) with writable paths, {len(findings)} "
            f"uncreated:\n",
            file=sys.stderr,
        )
        for f in sorted(set(findings)):
            print(f"  - {f}", file=sys.stderr)
        return 1

    print(
        f"{checked} unit(s) combine ProtectSystem=strict with ReadWritePaths, and "
        f"every path is created by tmpfiles, by the unit's own StateDirectory, or "
        f"by systemd itself. A path nothing creates fails the unit at namespace "
        f"setup, which only a fresh boot would show."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
