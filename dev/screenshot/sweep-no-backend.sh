#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Shoot every app's no-backend failure path, at desktop width, in one run.
#
# The set below was shot one panel at a time over several days, which is how it
# came to be shot entirely at 372 CSS px without anyone noticing: each run looked
# like the one before it. Enumerating the sweep makes two things possible that
# separate invocations did not - a width the whole set shares, and a summary at
# the end saying which panels have a current shot and which do not.
#
#   dev/screenshot/sweep-no-backend.sh              # everything, at 1280
#   dev/screenshot/sweep-no-backend.sh 1440         # everything, wider
#   dev/screenshot/sweep-no-backend.sh 1280 settings # one app's routes
#
# Slow by construction: each app is built for production first, because the
# fixtures are DEV-gated and a dev-server render would show sample data and prove
# nothing. Budget a couple of minutes per app.
#
# Each line is `<app> <route> <output-name> [click-selector]`. A route of `-` means
# the app's root; a fourth field is a CSS selector clicked before the shot.
#
# That fourth field is why knowledge's Library and Projects are in the list now.
# They switch inside a single route, so for as long as this path could not click,
# they were named here as the gap and never photographed. The first shot of
# Library found it printing "Cannot read your library right now" twice, once in
# the header line and once as the empty-list text.
set -uo pipefail

WIDTH="${1:-1280}"
ONLY="${2:-}"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

SHOTS=(
  "clock - clock-unavailable"
  "clock - clock-timers-unavailable #tab-timers"
  "clock - clock-focus-unavailable #tab-focus"
  "clock - clock-stopwatch-unavailable #tab-stopwatch"
  "clock - clock-world-unavailable #tab-world"
  "files - files-unavailable"
  "greeter - greeter-unavailable"
  "knowledge - knowledge-unavailable"
  "knowledge - knowledge-projects-unavailable button[data-place=projects]"
  "knowledge - knowledge-library-unavailable button[data-place=library]"
  "meetings - meetings-unavailable"
  "settings printers settings-printers-unavailable"
  "settings privacy settings-privacy-unavailable"
  "settings privacy/physical settings-sentinel-unavailable"
  "settings appearance/wallpaper settings-appearance-wallpaper-unavailable"
  "settings windows-apps settings-windows-apps-unavailable"
  "system-monitor - system-monitor-unavailable"
  "system-monitor - system-monitor-performance-unavailable #tab-performance"
  "terminal - terminal-unavailable"
  "text-editor - text-editor-unavailable"
  "viewers - viewers-unavailable"
)

ok=(); bad=()
for entry in "${SHOTS[@]}"; do
  read -r app route name click <<<"$entry"
  [ -n "$ONLY" ] && [ "$app" != "$ONLY" ] && continue
  [ "$route" = "-" ] && route=""
  out="$here/out/${name}.png"
  echo "=== $app ${route:-/} -> $name at ${WIDTH}px"
  if SHOOT_OPEN="${click:-}" "$here/shoot-no-backend.sh" "$app" "$route" "$out" "$WIDTH"; then
    ok+=("$name")
  else
    # Carry on rather than stop: one app failing to build should not cost the
    # other thirteen shots, and the summary below is what reports it.
    bad+=("$app ${route:-/}")
  fi
done

echo
echo "${#ok[@]} shot(s) written at ${WIDTH}px CSS."
if [ "${#bad[@]}" -gt 0 ]; then
  echo "${#bad[@]} did NOT produce a shot, so nothing about these panels is verified:"
  for b in "${bad[@]}"; do echo "  - $b"; done
  exit 1
fi
