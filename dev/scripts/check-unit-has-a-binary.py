#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A unit for a daemon the image does not build.

`check-shipped-units.py` asks the forward question: a unit exists, is there an
install line for it. This asks the reverse one, which nothing did: the unit's
`ExecStart` names a binary, and does anything in the image build actually produce
it. A unit whose binary is never built is a service that cannot start, and
systemd's answer to that is `status=203/EXEC` - "No such file or directory" with
no clue whose file.

WHY IT IS A DEFECT BY DEFAULT. A finished daemon ahead of the image is a real and
ordinary state, and this check does not demand that everything ship. It demands
that the answer be in the FILE. A unit is read by whoever is debugging a boot at
the time, and a reason living in a Python dict in `dev/scripts` is a reason they
will not find; worse, a carry list whose entries all say "presumed deliberate" is
a list that never shrinks, because nobody can tell which entries were ever
checked.

THE MARKER, and it has to be one so the gate can tell a stated reason from a unit
that merely has comments:

    # NOT ON THE IMAGE: <why, and what would change it>

in the unit, anywhere. Three units carry it today.

WHAT IS NOT ASKED. An `ExecStart` that is not one of ours - `busctl`, `sh`, a
systemd helper - belongs to the distribution, which ships it or does not
independently of anything in this tree. Only `arlen-*` binaries are the subject,
and `installd`'s trash-cleanup timer (which calls `busctl`) is the reason that
exemption is worth stating rather than leaving implied.

WHERE THE ANSWER COMES FROM. Two places, because the image builds binaries in
two: the `mkosi.build.d/*.chroot` phases, and the cross-build table in
`build-image.sh`. Reading only the first is how the event bus - which every other
daemon orders itself after - came back as "not built" on a measurement that was
wrong, and three units with it.
"""

try:
    import tracked_walk  # noqa: F401  the walk below reads what git knows about
except ModuleNotFoundError:  # a control's copy of this gate, run away from the module
    pass
import pathlib
import re
import sys

ROOT = pathlib.Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else pathlib.Path(__file__).resolve().parents[2]
MARKER = re.compile(r"^#\s*NOT ON THE IMAGE:\s*(\S.*)$", re.M)
EXEC = re.compile(r"^ExecStart=(\S+)", re.M)
SKIP_DIRS = {"node_modules", "target", ".git", "mkosi.tools", "mkosi.cache"}


def units() -> list[pathlib.Path]:
    """Every systemd unit this tree ships, from a component's `dist/` or an
    image staging directory. Filtered by content: the tree also holds D-Bus
    activation files called `*.service`, which have no `[Service]` section."""
    out = []
    for f in sorted(ROOT.rglob("*.service")):
        if SKIP_DIRS & set(f.parts) or not f.is_file():
            continue
        if not ({"dist", "systemd"} & set(f.parts)):
            continue
        if "[Service]" in f.read_text(encoding="utf-8", errors="replace"):
            out.append(f)
    return out


def build_text() -> str:
    """Everything the image build says, as one string to look a binary up in."""
    parts = []
    phases = ROOT / "dev" / "mkosi" / "mkosi.build.d"
    if phases.is_dir():
        parts += [p.read_text(encoding="utf-8", errors="replace") for p in sorted(phases.iterdir()) if p.is_file()]
    orchestrator = ROOT / "dev" / "mkosi" / "build-image.sh"
    if orchestrator.is_file():
        parts.append(orchestrator.read_text(encoding="utf-8", errors="replace"))
    return "\n".join(parts)


def main() -> int:
    found = units()
    if not found:
        print("check-unit-has-a-binary: no shipped unit found, so the scan is pointed wrong")
        return 1
    built = build_text()
    if not built:
        print("check-unit-has-a-binary: no image build found, so every unit would read as unbuilt")
        return 1

    problems, stated = [], []
    for f in found:
        m = EXEC.search(f.read_text(encoding="utf-8", errors="replace"))
        if not m:
            continue
        binary = m.group(1).lstrip("-").rsplit("/", 1)[-1]
        if not binary.startswith("arlen") and binary not in {"event-bus", "permission-helper"}:
            continue  # the distribution's, not ours
        if binary in built:
            continue
        text = f.read_text(encoding="utf-8", errors="replace")
        said = MARKER.search(text)
        rel = f.relative_to(ROOT)
        if said:
            stated.append((rel, binary, said.group(1).strip()))
        else:
            problems.append((rel, binary))

    for rel, binary in problems:
        print(f"{rel}: nothing in the image build produces `{binary}`, and the unit does not say why.")
        print(
            "    A unit whose binary is never built fails at spawn with 203/EXEC and no clue whose "
            "file is missing. Build it, or put `# NOT ON THE IMAGE: <why>` in the unit."
        )
    if stated:
        print("units deliberately ahead of the image, each saying so in the file:")
        for rel, binary, why in stated:
            print(f"  - {rel} ({binary}): {why}")
    if problems:
        print(f"\n{len(problems)} unit(s) for a binary the image does not build")
        return 1
    print(
        f"{len(found)} shipped unit(s): every one names a binary the image builds, or says in the "
        f"file why it does not ({len(stated)} of those)."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
