#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Shoot every app's failure paths, at desktop width, in one run.
#
# TWO PICTURES PER PANEL, and the pair is the point. `<name>.png` is the page with
# no runtime at all, which for a Tauri app is the browser preview: `tauriAvailable`
# is false, so every fixture guard fires and what you get is the demo the author
# put there deliberately. `<name>-hostfails.png` is the page a person actually
# meets - the runtime is there from the moment the webview loads and the COMMAND
# fails. Only the second reaches a fixture standing in for a failed read, and
# until 24 August this sweep could not render it at all: three apps were turning a
# thrown `launch_file` into "no document is open" and nothing here could see it.
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
  # The alarm editor, which the chrome "+" opens. Both the new-alarm form and
  # the editor for an existing one, and until it was shot it said neither.
  "clock - clock-alarm-editor #chrome-add"
  "clock - clock-timers-unavailable #tab-timers"
  "clock - clock-focus-unavailable #tab-focus"
  "clock - clock-stopwatch-unavailable #tab-stopwatch"
  "clock - clock-world-unavailable #tab-world"
  "files - files-unavailable"
  "files - files-recent-unavailable [data-place=recent]"
  "files - files-trash-unavailable [data-place=trash]"
  "greeter - greeter-unavailable"
  "knowledge - knowledge-unavailable"
  "knowledge - knowledge-projects-unavailable button[data-place=projects]"
  "knowledge - knowledge-library-unavailable button[data-place=library]"
  "knowledge - knowledge-searches-unavailable button[data-place=searches]"
  "meetings - meetings-unavailable"
  # Two of this app's three routes had never been shot. The capture surface is
  # where the red dot and Stop live; the note route is where a stale link lands,
  # and it is the one that says "this meeting" rather than repeating the list's
  # sentence - worth keeping photographed so it stays that way.
  "meetings capture meetings-capture-unavailable"
  "meetings meeting/abc meetings-note-missing"
  # NB not in this list: the refused-STOP line. It needs a capture that started
  # and then a stop that failed, which no single no-backend session produces, so
  # it is reachable only through the dev-only pin the viewers app also carries:
  #   npx vite dev -C apps/meetings   then   /capture?state=stop-failed
  # The five pages that gained `ConfigUnavailable` on 16 August, none of which
  # had a shot: the fix says the controls below are defaults rather than your
  # settings, and that sentence is the whole repair, so it is the thing to keep
  # a picture of.
  "settings focus settings-focus-unavailable"
  "settings accessibility settings-accessibility-unavailable"
  "settings workspaces settings-workspaces-unavailable"
  "settings system-actions settings-system-actions-unavailable"
  "settings knowledge settings-knowledge-unavailable"
  "settings printers settings-printers-unavailable"
  "settings keyboard settings-keyboard-unavailable"
  # The list under the toolbar. It used to answer a failed read with "No bindings
  # match """, blaming a search box nobody had typed in, so this shot is of the
  # sentence rather than of the (correctly empty) list.
  "settings keyboard/shortcuts settings-shortcuts-unavailable"
  # The add-binding form only exists once its dialog is open, which is why it was
  # never in a shot and its six labels stayed English until 17 August.
  "settings keyboard/shortcuts settings-shortcuts-dialog [data-action=add-custom]"
  # A destructive confirm nobody had looked at: it offers to reset every binding
  # on a page that has just said it cannot read them.
  "settings keyboard/shortcuts settings-shortcuts-reset [data-action=reset-all]"
  "settings privacy settings-privacy-unavailable"
  "settings privacy/physical settings-sentinel-unavailable"
  "settings appearance/wallpaper settings-appearance-wallpaper-unavailable"
  "settings appearance/quicksettings settings-quicksettings-unavailable"
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
  if SHOOT_OPEN="${click:-}" SHOOT_FAILING_HOST=both \
      "$here/shoot-no-backend.sh" "$app" "$route" "$out" "$WIDTH"; then
    ok+=("$name")
  else
    # Carry on rather than stop: one app failing to build should not cost the
    # other thirteen shots, and the summary below is what reports it.
    bad+=("$app ${route:-/}")
  fi
done

echo
echo "${#ok[@]} panel(s) at ${WIDTH}px CSS, each as a pair: the plain name is the"
echo "no-runtime path, the -hostfails one is the runtime-there-backend-broken path."
if [ "${#bad[@]}" -gt 0 ]; then
  echo "${#bad[@]} did NOT produce a shot, so nothing about these panels is verified:"
  for b in "${bad[@]}"; do echo "  - $b"; done
  exit 1
fi
