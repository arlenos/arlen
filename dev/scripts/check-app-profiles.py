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

It also checks the two things that decide whether a present profile actually
loads: the file parses as TOML, and its `[info] app_id` is the id its filename
claims. A profile naming a different app is the drift the file-manager's own
header warned about ("confirm it matches the daemon's path_to_app_id resolution
of the installed binary"), and it fails at launch rather than here.

What it does NOT check:

  * whether the profile GRANTS the right things. A profile that parses and names
    the app is what makes it start; whether its scopes match what the app reads
    is a per-app judgement, and each shipped profile argues for its own in its
    header.
  * the profile catalogue under `sdk/permissions/profiles/` - 2273 authored
    profiles for third-party applications. Those are staged by
    `mkosi.build.d/08p-profiles.sh.chroot` into `/usr/share/arlen/profiles`,
    which is where the apt-enrolment hook matches them by package name; they are
    a different thing from the per-app profiles compared here, and they become an
    app's profile only when a package install enrols one.
  * apps that are not in the image at all.

Shown to fail before being trusted: deleting a shipped profile, or adding an app
step without one, makes it name that app.
"""

import re
import sys
import tomllib
from pathlib import Path

# Takes the tree to scan as an argument so the check can be pointed at a
# fixture and SHOWN TO FAIL. It had no positive control because it could not
# be handed one: a hardcoded root can only ever be run against a tree that
# passes, which is the same as never having seen it speak.
ROOT = (
    Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
)
STEPS = ROOT / "dev/mkosi/mkosi.build.d"
PROFILES = ROOT / "dev/mkosi/mkosi.extra/var/lib/arlen/permissions/1000"

# The apps installed without a profile, and why. EMPTY as of 9 August: every app
# the image installs has one. Each entry was a promise that an app could not be
# launched confined yet, so an empty list is the state this was written to reach -
# keep it empty by writing the profile rather than the excuse.
# The webview-directory blocker that used to sit here is CLOSED: the launcher sets
# `XDG_DATA_HOME` to `~/.local/share/arlen/apps`, so a Tauri app's `appDataDir()`
# (`$XDG_DATA_HOME/<bundle identifier>`) lands on the granted directory instead of
# an ungranted `~/.local/share/<id>`. Decided as move-not-grant, and it works out
# to one directory because the app id and the bundle identifier are now the same
# string.
PENDING: dict[str, str] = {}

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
    for path in sorted(PROFILES.glob("*.toml")) if PROFILES.is_dir() else []:
        try:
            declared = tomllib.loads(path.read_text(encoding="utf-8"))
        except (OSError, tomllib.TOMLDecodeError) as e:
            problems.append(f"{path.relative_to(ROOT)} does not parse ({e}), so the launcher refuses the app")
            continue
        named = declared.get("info", {}).get("app_id")
        if named != path.stem:
            problems.append(
                f"{path.relative_to(ROOT)} is named for {path.stem} and its "
                f"`[info] app_id` says {named!r}. The launcher looks the file up by "
                f"the id and then trusts what is inside it, so these must agree."
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
        f"{len(PENDING)} pending with a reason; each present profile parses and "
        f"names its own app. An empty pending list is the state a confined launch "
        f"needs, so keep it empty by writing the profile rather than the excuse."
    )
    if problems:
        print("\ninstalled without a profile:\n")
        for p in problems:
            print(f"  - {p}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
