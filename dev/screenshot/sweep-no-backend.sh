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
# Each line is `<app> <route> <output-name>`. A route of `-` means the app's root.
# Two shots that used to exist are not here: knowledge's Library and Projects
# views are switched inside a single route, and this path cannot drive a click,
# so shooting them would need the WebDriver route's `--open`. Named rather than
# dropped quietly.
set -uo pipefail

WIDTH="${1:-1280}"
ONLY="${2:-}"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

SHOTS=(
  "clock - clock-unavailable"
  "files - files-unavailable"
  "greeter - greeter-unavailable"
  "knowledge - knowledge-unavailable"
  "meetings - meetings-unavailable"
  "settings printers settings-printers-unavailable"
  "settings privacy settings-privacy-unavailable"
  "settings privacy/physical settings-sentinel-unavailable"
  "settings appearance/wallpaper settings-appearance-wallpaper-unavailable"
  "settings windows-apps settings-windows-apps-unavailable"
  "system-monitor - system-monitor-unavailable"
  "terminal - terminal-unavailable"
  "text-editor - text-editor-unavailable"
  "viewers - viewers-unavailable"
)

ok=(); bad=()
for entry in "${SHOTS[@]}"; do
  read -r app route name <<<"$entry"
  [ -n "$ONLY" ] && [ "$app" != "$ONLY" ] && continue
  [ "$route" = "-" ] && route=""
  out="$here/out/${name}.png"
  echo "=== $app ${route:-/} -> $name at ${WIDTH}px"
  if "$here/shoot-no-backend.sh" "$app" "$route" "$out" "$WIDTH"; then
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
