# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that one application gets one answer, whatever it was packaged as.

An app appears in the corpus under every id it ships under: `konsole` and
`org.kde.konsole`, `ghostty` and `com.ghostty.Ghostty` and
`com.mitchellh.ghostty`, `signal` and `signal-desktop` and `org.signal.Signal`.
They are the same program. When their grants differ, what the app may touch
depends on where the user got it, which is not a decision anybody made.

Found on 17 August by reading the terminal profiles: `ghostty` had Home and no
network while its two Flatpak ids had Home AND network, and `konsole` disagreed
with `org.kde.konsole` the same way. Generalising it turned up ~99 more, one of
them written an hour earlier by the person generalising it - `clamtk` was given
`home = true` while `com.gitlab.davem.ClamTk` grants no filesystem at all and
says why (its scan paths are portal-selected).

BASELINE-DIFF, not a wall. The existing disagreements are real work with real
judgement in them - a Flatpak reaching files through portals genuinely warrants a
narrower static grant than the distro build - so they are recorded rather than
fixed under time pressure, and this gate holds the line at NEW ones. Same shape
as the born-translatable lint.

What it deliberately does NOT do: decide which side is right. It reports that a
group disagrees and leaves the answer to somebody who knows the app.
"""

import re
import sys
import tomllib
from collections import defaultdict
from pathlib import Path

# Flags first, so `--update` is never mistaken for the optional root path.
_ARGS = [a for a in sys.argv[1:] if not a.startswith("-")]
ROOT = Path(_ARGS[0]).resolve() if _ARGS else Path(__file__).resolve().parents[2]
PROFILES = ROOT / "sdk/permissions/profiles"
BASELINE = ROOT / "dev/profile-agreement-baseline.tsv"

#: Last segments that name a CATEGORY rather than a program. Without these,
#: `com.brave.Browser` and `io.gitlab.librewolf-community.browser` group as one
#: app, and the three `*.Client` ids (Dropbox, Skype, Spotify) as another - three
#: unrelated programs reported as disagreeing with each other.
GENERIC = {
    "browser", "client", "app", "desktop", "player", "editor", "viewer",
    "manager", "studio", "console", "terminal", "shell", "launcher", "tool",
}

#: Packaging suffixes that decorate a name without changing the program.
SUFFIX = re.compile(r"-(desktop|ce|bin|git|gtk|qt|kde|gnome|nightly|beta)$")

#: Desktop-environment prefixes a distro build carries and the upstream id does
#: not: `gnome-obfuscate` is `com.belmoussaoui.Obfuscate`. Without this the two
#: are never compared, which is how Obfuscate held the whole home tree under one
#: id and Pictures under the other for as long as both existed.
VENDOR = re.compile(r"^(gnome|kde|xfce|mate|deepin|elementary)-")


def core(stem: str) -> str | None:
    """The program a profile id names, or None if the id says only a category."""
    tail = stem.split(".")[-1].lower()
    name = VENDOR.sub("", SUFFIX.sub("", tail)).replace("_", "-")
    # The category check comes AFTER stripping, not before. `gnome-terminal` is
    # not in GENERIC as written, but it becomes `terminal` once the prefix is
    # gone, and grouping every terminal emulator as one program would report a
    # dozen unrelated apps as disagreeing with each other.
    if name in GENERIC:
        return None
    return name


def grants(doc: dict) -> tuple:
    """Everything a profile actually hands out, order-independent."""
    fs = tuple(sorted(k for k, v in doc.get("filesystem", {}).items() if v is True))
    net = bool(doc.get("network", {}).get("allow_all"))
    clip = tuple(sorted(k for k, v in doc.get("clipboard", {}).items() if v is True))
    return (fs, net, clip)


def main() -> int:
    if not PROFILES.is_dir():
        print(f"NOTHING WAS READ: no profiles at {PROFILES}", file=sys.stderr)
        return 2

    groups: dict[str, list[tuple[str, tuple]]] = defaultdict(list)
    read = 0
    for path in sorted(PROFILES.glob("*.toml")):
        name = core(path.stem)
        if name is None:
            continue
        try:
            doc = tomllib.load(path.open("rb"))
        except tomllib.TOMLDecodeError as e:
            print(f"{path.name}: does not parse ({e})", file=sys.stderr)
            return 1
        read += 1
        groups[name].append((path.stem, grants(doc)))

    if not read:
        print("NOTHING WAS READ: no profile carried a usable id", file=sys.stderr)
        return 2

    disagree = {
        name: sorted(members)
        for name, members in groups.items()
        if len(members) > 1 and len({g for _, g in members}) > 1
    }

    known = set()
    if BASELINE.is_file():
        known = {
            line.split("\t")[0]
            for line in BASELINE.read_text(encoding="utf-8").splitlines()
            if line.strip() and not line.startswith("#")
        }

    if "--update" in sys.argv:
        lines = ["# One app, one answer: groups whose packaging ids grant different things.",
                 "# Recorded so the gate can hold the line at NEW ones. Each is real work:",
                 "# decide what the app should get, make every id say it, drop the row.",
                 "#",
                 "# app\tthe ids that disagree"]
        for name, members in sorted(disagree.items()):
            lines.append(f"{name}\t" + " ".join(s for s, _ in members))
        BASELINE.write_text("\n".join(lines) + "\n", encoding="utf-8")
        print(f"baseline updated with {len(disagree)} group(s) -> {BASELINE}")
        return 0

    fresh = {n: m for n, m in disagree.items() if n not in known}
    # A row for a group that now agrees is a claim about the tree that stopped
    # being true. Reported, not failed: the fix is a `--update`, and nobody
    # should be blocked because they resolved one.
    stale = sorted(known - set(disagree))
    if stale:
        print(
            f"\n{len(stale)} recorded group(s) now agree and can leave the baseline "
            f"(`--update`): {', '.join(stale)}"
        )
    print(
        f"{read} profile(s) in {len(groups)} app group(s); "
        f"{len(disagree)} disagree, {len(known)} recorded. "
        f"A group is several packaging ids for ONE program, so a difference here means "
        f"the app's grants depend on where it was installed from."
    )
    if fresh:
        print("\napps whose packaging ids now disagree:\n", file=sys.stderr)
        for name, members in sorted(fresh.items()):
            print(f"  {name}:", file=sys.stderr)
            for stem, (fs, net, clip) in members:
                bits = [f"fs={list(fs)}", f"net={net}"] + ([f"clipboard={list(clip)}"] if clip else [])
                print(f"      {stem:<38} {'  '.join(bits)}", file=sys.stderr)
        print(
            "\nGive the app one answer and make every id say it, or run with --update "
            "if the difference is deliberate (a Flatpak reaching files through portals "
            "is the case that usually is).",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
