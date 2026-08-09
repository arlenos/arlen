# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that every app the image installs has a permission profile, or says why not.

A confined launch resolves the app's id from where its binary lives and looks for
`/var/lib/arlen/permissions/<uid>/<id>.toml`. `arlen-run` treats a missing profile
as a refusal - exit 65, never a fall back to unconfined, which is the right rule.
So an app the image installs without a profile is an app that will not start the
day the confinement flag goes true, and nothing says so until then.

That is not hypothetical: the flag's remaining work was discovered by hand on
9 August, and the count ("five profiles, five apps, and they do not line up")
had to be re-derived from scratch each time somebody asked. It is data in the
tree on both sides - the mkosi steps say which apps are installed, the staged
`mkosi.extra` tree says which profiles ship - so it can be compared instead.

What this checks: every app staged by a `dev/mkosi/mkosi.build.d/*.chroot` step
has a profile file named for the id it resolves to, or an entry in PENDING below
with the reason it does not have one yet.

What it does NOT check:

  * whether the profile GRANTS the right things. A profile that parses and names
    the app is what makes it start; whether its scopes match what the app reads
    is a per-app judgement, and each shipped profile argues for its own in its
    header.
  * the profile catalogue under `sdk/permissions/profiles/` - 2273 authored
    profiles for third-party applications that no install step reaches. That is
    a packaging gap, tracked separately, and it is not what this compares.
  * apps that are not in the image at all.

Shown to fail before being trusted: deleting a shipped profile, or adding an app
step without one, makes it name that app.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
STEPS = ROOT / "dev/mkosi/mkosi.build.d"
PROFILES = ROOT / "dev/mkosi/mkosi.extra/var/lib/arlen/permissions/0"

# The apps installed without a profile, and why. Each entry is a promise that the
# app cannot be launched confined yet, so the list should shrink to nothing before
# the confinement flag is flipped - which is exactly what makes it worth writing
# down rather than rediscovering.
# NB the pending list is not the only thing between here and a confined desktop.
# Measured on 9 August with an empty profile: a confined app can write its granted
# `~/.local/share/arlen/apps/<id>`, and `~/.local/share/<identifier>` - where a
# Tauri app's webview actually keeps WebKitCache, CacheStorage and its storage - is
# neither visible nor creatable. That applies to ALL SIX installed apps, the three
# WITH profiles included, so having a profile is necessary and not sufficient.
# Either the launcher grants that directory too or the apps are pointed at the
# granted one; both are decisions. See the report of 9 August.
PENDING: dict[str, str] = {
    "dev.arlen.clock": "needs nothing from the graph, but its webview data lives outside the granted app dirs (see above)",
    "dev.arlen.meetings": "renders from a labelled fixture, so it needs no scope - same webview-directory question as the others",
    # Measured, not assumed: a confined run with an empty profile reaches /proc
    # (bwrap mounts a private procfs) and NOT /sys, and `/sys` is on the launcher's
    # FORBIDDEN_FS_ROOTS, so a `custom` grant for it is dropped by design. The app
    # reads both. What it needs is a read-only grant of a /sys SUBTREE, and the
    # profile format's `custom` binds read-write - so this one waits on a format
    # question, not on somebody writing a file.
    "dev.arlen.system-monitor": "the read-only subtree grant it needed now exists (`[filesystem] read_only`), so this one is down to writing the file and the webview-directory question above",
}

INSTALL = re.compile(r'\$DESTDIR/usr/lib/arlen/apps/([A-Za-z0-9._-]+)/bin/')


def installed_apps() -> dict[str, str]:
    """App id to the build step that installs it."""
    out: dict[str, str] = {}
    for step in sorted(STEPS.glob("*.chroot")):
        for m in INSTALL.finditer(step.read_text(encoding="utf-8", errors="replace")):
            out[m.group(1)] = step.name
    return out


def main() -> int:
    apps = installed_apps()
    if not apps:
        print("no app install steps found; the layout moved and this check did not")
        return 1
    shipped = {p.stem for p in PROFILES.glob("*.toml")} if PROFILES.is_dir() else set()

    problems: list[str] = []
    for app, step in sorted(apps.items()):
        if app in shipped or app in PENDING:
            continue
        problems.append(
            f"{app} is installed by {step} and ships no profile. A confined launch "
            f"refuses it (arlen-run exits 65 on a missing profile). Add "
            f"{PROFILES.relative_to(ROOT)}/{app}.toml, or add it to PENDING with the "
            f"reason it cannot have one yet."
        )
    for app, reason in sorted(PENDING.items()):
        if app in shipped:
            problems.append(f"{app} is listed as pending and also ships a profile; drop the PENDING entry")
        elif app not in apps:
            problems.append(f"{app} is listed as pending but the image does not install it; drop the entry")
        elif not reason.strip():
            problems.append(f"{app} is pending with no reason given")

    ready = sum(1 for a in apps if a in shipped)
    print(
        f"{len(apps)} app(s) installed by the image: {ready} with a permission profile, "
        f"{len(PENDING)} pending with a reason. The pending list has to reach zero "
        f"before a confined launch works for everything the image ships."
    )
    if problems:
        print("\ninstalled without a profile:\n")
        for p in problems:
            print(f"  - {p}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
