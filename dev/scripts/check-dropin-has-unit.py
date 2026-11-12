#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""A drop-in directory for a unit the image does not have.

systemd applies `<unit>.d/*.conf` to the unit of exactly that name in exactly
that scope. Name a unit that is not there and nothing happens: no error, no log
line, the settings are simply never read.

THAT IS HOW THE BUS RAN UNENFORCED. `ARLEN_EVENT_BUS_ENFORCE=1` was set in
`system/arlen-event-bus.service.d/10-enforce.conf` and nowhere else. On 15 August
the bus was split into `system/arlen-event-bus-system.service` and
`user/arlen-event-bus.service`; the drop-in kept its old name, matched neither,
and both buses ran in shadow mode for three months while the documents said
enforced. Found on 11 November by reading, not by anything failing.

WHAT COUNTS AS THE UNIT BEING THERE. One of three, in the same scope:

  * the unit file is staged beside the drop-in, under
    `mkosi.extra/usr/lib/systemd/<scope>/` or `mkosi.extra/etc/systemd/<scope>/`;
  * a build phase installs it, which reads as the path
    `systemd/<scope>/<unit>` in `mkosi.build.d/` or `build-image.sh`;
  * it is the distribution's, and named in DISTRIBUTION below with the package
    that ships it.

WHAT IS NOT ASKED. A `*.conf.d` directory (`logind.conf.d`) configures a daemon,
not a unit, and has no unit to match. Whether the drop-in's CONTENT is right is
not this check's question either.

Run: dev/scripts/check-dropin-has-unit.py [root]
"""

try:
    import tracked_walk  # noqa: F401  the walk below reads what git knows about
except ModuleNotFoundError:  # a control's copy of this gate, run away from the module
    pass
import pathlib
import sys

ROOT = pathlib.Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else pathlib.Path(__file__).resolve().parents[2]
EXTRA = ROOT / "dev" / "mkosi" / "mkosi.extra"
BASES = ("usr/lib/systemd", "etc/systemd")
SCOPES = ("system", "user")
KINDS = (".service", ".socket", ".timer", ".target", ".path", ".mount", ".slice")

#: Units a Debian package ships, so no file of ours stands behind the drop-in.
DISTRIBUTION = {
    ("system", "greetd.service"): "the greetd package",
}


def build_text() -> str:
    """Everything the image build says, to look an installed unit path up in."""
    parts = []
    phases = ROOT / "dev" / "mkosi" / "mkosi.build.d"
    if phases.is_dir():
        parts += [p.read_text(encoding="utf-8", errors="replace") for p in sorted(phases.iterdir()) if p.is_file()]
    orchestrator = ROOT / "dev" / "mkosi" / "build-image.sh"
    if orchestrator.is_file():
        parts.append(orchestrator.read_text(encoding="utf-8", errors="replace"))
    return "\n".join(parts)


def dropins() -> list[tuple[str, str, pathlib.Path]]:
    """Every `<unit>.d` directory staged for the image, as (scope, unit, path)."""
    found = []
    for base in BASES:
        for scope in SCOPES:
            where = EXTRA / base / scope
            if not where.is_dir():
                continue
            for d in sorted(where.iterdir()):
                if d.is_dir() and d.name.endswith(".d") and d.name[:-2].endswith(KINDS):
                    found.append((scope, d.name[:-2], d))
    return found


def main() -> int:
    """Refuse a drop-in directory whose unit is not on the image in its scope."""
    if not EXTRA.is_dir():
        print(f"check-dropin-has-unit: no {EXTRA.relative_to(ROOT)}, so the scan is pointed wrong")
        return 2
    found = dropins()
    if not found:
        print("check-dropin-has-unit: no drop-in directory found, and the image has had one since August")
        return 2
    built = build_text()
    problems = []
    for scope, unit, path in found:
        staged = any((EXTRA / base / scope / unit).is_file() for base in BASES)
        installed = f"systemd/{scope}/{unit}" in built
        if staged or installed or (scope, unit) in DISTRIBUTION:
            continue
        elsewhere = [s for s in SCOPES if s != scope and any((EXTRA / b / s / unit).is_file() for b in BASES)]
        hint = f" A unit of that name exists in the {elsewhere[0]} scope, which this does not reach." if elsewhere else ""
        problems.append(f"  - {path.relative_to(ROOT)}: no {scope} unit named {unit} is on the image.{hint}")
    if problems:
        print("Drop-in directories systemd will apply to nothing:\n")
        print("\n".join(problems))
        print(
            "\nRename the directory after the unit it is for, in that unit's scope, or name the\n"
            "package that ships the unit in DISTRIBUTION."
        )
        return 1
    print(f"check-dropin-has-unit: {len(found)} drop-in directories, each beside a unit of its name")
    return 0


if __name__ == "__main__":
    sys.exit(main())
