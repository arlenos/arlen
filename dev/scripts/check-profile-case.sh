#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Report permission profiles that a real desktop entry can never reach because
# only their CASE differs.
#
# A foreign app's id is its freedesktop desktop-id, which is the `.desktop`
# basename verbatim - `derive_app_id` in the shell keeps the case, and its own
# doc example is `org.gnome.Calculator`. The profile is then looked up as an
# exact `{app_id}.toml`. So a profile written as `cinny.toml` is invisible to
# `Cinny.desktop`, and `arlen-run` refuses the launch with "profile not found"
# (exit 65) even though somebody wrote that profile deliberately.
#
# Fail-closed, so this is not a hole - it is worse in a quieter way: the app
# simply will not start once confinement is on, and the corpus looks complete.
# Six turned up on the first machine this was pointed at, out of 184 installed
# entries against 2273 profiles.
#
# NOT a CI gate: it needs a machine with apps installed, and CI has none. Run it
# wherever real software lives.
#
#   dev/scripts/check-profile-case.sh [desktop-dir ...]

set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
PROFILES="$ROOT/sdk/permissions/profiles"
[ -d "$PROFILES" ] || { echo "no profiles at $PROFILES" >&2; exit 2; }

dirs=("$@")
if [ ${#dirs[@]} -eq 0 ]; then
    dirs=(/usr/share/applications "$HOME/.local/share/applications"
          /var/lib/flatpak/exports/share/applications)
fi

checked=0
findings=0
for dir in "${dirs[@]}"; do
    [ -d "$dir" ] || continue
    for entry in "$dir"/*.desktop; do
        [ -e "$entry" ] || continue
        id=$(basename "$entry" .desktop)
        checked=$((checked + 1))
        # Exact hit is the only thing the resolver will find.
        [ -f "$PROFILES/$id.toml" ] && continue
        lower=$(printf '%s' "$id" | tr '[:upper:]' '[:lower:]')
        if [ "$lower" != "$id" ] && [ -f "$PROFILES/$lower.toml" ]; then
            printf '  %s.desktop is unreachable: the profile is %s.toml\n' "$id" "$lower"
            findings=$((findings + 1))
        fi
    done
done

# A run that read nothing is not a pass. The whole point is comparing against
# real installed software, so no entries means no evidence either way.
if [ "$checked" -eq 0 ]; then
    echo "NOTHING WAS READ: no .desktop entries under ${dirs[*]}" >&2
    exit 2
fi

echo "$checked desktop entry/entries checked against $(ls "$PROFILES" | wc -l) profiles"
if [ "$findings" -gt 0 ]; then
    echo "$findings profile(s) that exist and cannot be found." >&2
    echo "Each is a written, correct profile the launcher will answer with" >&2
    echo "\"profile not found\" once confinement is on." >&2
    exit 1
fi
echo "no profile is hidden behind a case difference on this machine"
