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

import os
import pathlib
import re
import sys

ROOT = (
    pathlib.Path(sys.argv[1]).resolve()
    if len(sys.argv) > 1
    else pathlib.Path(__file__).resolve().parents[2]
)

#: Paths that exist without anyone creating them. Deliberately short, and they
#: are whole paths rather than prefixes.
#:
#: `%h` was in here as a PREFIX, on the reasoning that a user's home is theirs to
#: write in. That is true of the home and false of everything inside it, and the
#: gate passed `installd.service` because of it - the 19 Aug boot log has the same
#: NAMESPACE death it was written to catch, one unit further down:
#:
#:     installd.service: Failed to set up mount namespacing:
#:       /home/arlen/.local/share/applications: No such file or directory
#:
#: Second time this check has been wrong in the permissive direction. The pattern
#: both times was accepting an ancestor as proof about a descendant.
GUARANTEED_EXACT = ("%t", "/run/user/%U", "%h", "/tmp", "/var/tmp")

DIRECTIVE = re.compile(r"^(\w+)=(.*)$")


#: Where OUR tmpfiles live. Globbing for `**/tmpfiles.d/*.conf` also finds
#: Debian's own, vendored under the mkosi cache and tools trees - which is how
#: the first version of this gate decided `/var/lib` was created (by apt's file)
#: and therefore that everything under it was fine.
OURS = ("dev/mkosi/mkosi.extra", "daemons", "apps", "distro")


def tmpfiles_paths(root: pathlib.Path) -> set[str]:
    """Every path a tmpfiles.d entry WE ship creates."""
    out: set[str] = set()
    # PRUNED rather than filtered afterwards. `Path.glob("**/...")` descends into
    # every directory first and the caller drops the matches later, so the walk
    # covered `target/` and `mkosi.builddir/` - gigabytes, on every commit, while
    # a build may be writing them. Skipping them at the directory level is the
    # same answer for a fraction of the work, and it stops a gate's verdict
    # depending on what a concurrent build happens to be doing.
    skip = {"target", "mkosi.builddir", "node_modules", "mkosi.cache", "mkosi.tools", ".git"}
    confs: list[pathlib.Path] = []
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in skip]
        if pathlib.Path(dirpath).name == "tmpfiles.d":
            confs.extend(pathlib.Path(dirpath) / f for f in filenames if f.endswith(".conf"))
    for conf in confs:
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


#: The image's one user. `%h` in a user unit resolves to this, and the tmpfiles
#: file and the staged home tree both spell it out, so a comparison that leaves
#: the specifier unexpanded matches nothing and the gate has to fall back on
#: prefix reasoning - which is how it passed `installd.service`.
IMAGE_HOME = "/home/arlen"

#: `%t` and `/run/user/%U` are the same directory written two ways.
RUNTIME_ROOT = "/run/user/%U"


def expand(path: str) -> str:
    """Resolve the specifiers a unit may write, so paths compare as paths."""
    path = path.rstrip("/")
    if path.startswith("%h"):
        return IMAGE_HOME + path[2:]
    if path.startswith("%t"):
        return RUNTIME_ROOT + path[2:]
    return path


def staged_home(root: pathlib.Path) -> set[str]:
    """Directories the `mkosi.extra` tree places into the image.

    A file staged at `mkosi.extra/home/arlen/.config/arlen/ai.toml` makes
    `/home/arlen/.config/arlen` exist on the image, which is what satisfies
    `ReadWritePaths=%h/.config/arlen` for the wallpaper and settings-broker units.
    Nothing else creates that directory, so leaving this source out would report
    two working units as broken.
    """
    out: set[str] = set()
    extra = root / "dev/mkosi/mkosi.extra"
    if not extra.is_dir():
        return out
    for node in extra.rglob("*"):
        rel = "/" + node.relative_to(extra).as_posix()
        for parent in pathlib.PurePosixPath(rel).parents:
            if str(parent) != "/":
                out.add(str(parent))
        if node.is_dir():
            out.add(rel)
    return out


def shared_runtime_dirs(all_units: list[pathlib.Path]) -> set[str]:
    """Runtime directories one unit declares and the others rely on.

    `/run/user/<uid>/arlen` holds every arlen socket, so it is deliberately NOT
    each daemon's own `RuntimeDirectory=` - systemd would delete it when that one
    daemon stopped. A few units declare it with `RuntimeDirectoryPreserve=yes` and
    the rest name it in `ReadWritePaths=`, which is what makes them start today.

    This pools those declarations, and the pooling carries an assumption worth
    naming: it says SOMETHING creates the directory, not that it is created before
    the unit that needs it. Ordering is the units' own business and several of them
    discuss it in their comments.
    """
    out: set[str] = set()
    for unit in all_units:
        for line in unit.read_text().splitlines():
            m = DIRECTIVE.match(line.strip())
            if not m or m.group(1) != "RuntimeDirectory":
                continue
            for v in m.group(2).split():
                out.add(f"{RUNTIME_ROOT}/{v}")
                out.add(f"/run/{v}")
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
    path = expand(path)
    if path in {expand(g) for g in GUARANTEED_EXACT}:
        return True
    return path in {expand(c) for c in creators}


def main() -> int:
    all_units = units(ROOT)
    if not all_units:
        print(f"NOTHING WAS READ: no unit files under {ROOT}", file=sys.stderr)
        return 2

    made_by_tmpfiles = (
        tmpfiles_paths(ROOT)
        | staged_dirs(ROOT)
        | staged_home(ROOT)
        | shared_runtime_dirs(all_units)
    )
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
