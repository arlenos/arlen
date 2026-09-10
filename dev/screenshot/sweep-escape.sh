#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Does Escape close what a click opened? One row per overlay, over a running app.
#
# WHY THIS IS A SWEEP AND NOT A READING. `escape-dismisses.js` exists because on
# 8 September a Settings dialog mounted its Escape handler on its own backdrop,
# which only receives a key press when focus is already inside it - so every
# reader of that file, including the one who wrote it, saw a dialog that handled
# Escape, and driven it answered `before=[dialog] after=[dialog]`. Since then the
# probe has been a hand tool nothing ran, which means the property is checked
# whenever somebody remembers it. This is the list, and what is missing from the
# list is invisible rather than clean - the same claim `sweep-render-all.sh`
# makes about its own table.
#
# WHAT COUNTS AS A FAILURE, and there are two of them:
#
#   still open   the overlay is on screen after the key. The defect.
#   opened none  the click landed and this probe saw no overlay at all, so the
#                row proves nothing. Reported rather than passed, because a
#                selector that stopped opening anything is exactly how a sweep
#                goes quietly green.
#
# NOT EVERY `::selector` ROW IN THE RENDER TABLE BELONGS HERE. Most of them open
# a tab, a sidebar or a list row, and Escape closing a tab would be wrong. The
# rows below are overlays: something a person opens ON TOP of the surface and
# expects to be able to dismiss.
#
# THE SERVER IS THE CALLER'S. Same layering as `sweep-render.sh`: this takes a
# base URL so it can run against `npm run dev` or against whatever
# `sweep-render-all.sh` already started, and does not grow a second copy of the
# dev-server lifecycle. Two copies of a careful thing drift, which is how the two
# render runners ended up waiting different amounts for the same selector.
#
#   dev/screenshot/sweep-escape.sh http://localhost:1420 \
#     '[data-applet-id=network]' '[data-applet-id=audio]'
#
# With no selectors it runs the shell's applet set, which is the case this was
# written for.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

base="${1:-}"
[ -n "$base" ] || { echo "usage: sweep-escape.sh <base-url> [selector...]" >&2; exit 2; }
shift

# The shell's bar panels, which is the set a person opens from the top bar. Read
# off `sweep-render-all.sh`'s desktop-shell row rather than invented, so the two
# tables name the same things - INCLUDING the `@@host` suffix, which four of them
# carry and which this script therefore has to understand.
#
# FOUR OF THEM DO NOT EXIST WITHOUT THEIR FIXTURE, and I learnt that by running
# this rather than by reading the table: `[data-applet-id=audio]` refused with
# "matched no element in 5s", because the audio, tray, battery and media applets
# only render when there is something to show and a no-backend shell has nothing.
# Their host scripts supply it. Without the suffix those four rows would have read
# as a broken selector for as long as nobody looked.
#
# WITH NO SELECTORS the set is chosen from the ROUTE the base URL already names,
# so what has been driven stays driven rather than living in whoever's shell
# history ran it. Three settings dialogs are here for a specific reason: the
# window-rule dialog is the surface this whole probe was written for - it shipped
# with its Escape handler on its own backdrop, which never receives the key - and
# it now passes, which is a fact worth being able to re-establish in one command
# rather than by remembering a selector.
case "$base" in
  */workspaces|*/workspaces?*)
    [ "$#" -eq 0 ] && set -- '.footer button' ;;
  */keyboard/shortcuts|*/keyboard/shortcuts?*)
    [ "$#" -eq 0 ] && set -- '[data-action=add-custom]' '[data-action=reset-all]' ;;
  # The reset-to-defaults confirm. `button[data-slot=button]` rather than an id
  # because the page has none, and it is unambiguous: of the six visible buttons
  # on that route five are window chrome and the sidebar rail, and this is the
  # only one the kit's Button renders. Read off the DOM, like every selector in
  # `sweep-render-all.sh`, not guessed.
  */appearance/quicksettings|*/appearance/quicksettings?*)
    [ "$#" -eq 0 ] && set -- 'button[data-slot=button]' ;;
  # THE PROVIDER FORM, and the reason it is the only one of the six remaining
  # Settings dialogs named here. System actions, physical privacy, the app page
  # and the windows-app page all open the KIT's `ConfirmDialog` - one component,
  # already driven twice above (`reset-all`, and the quick-settings reset). Five
  # more rows would re-prove one implementation and would read as coverage the
  # sweep does not have. `AddProviderDialog` is the app's own form dialog and
  # nothing has ever pressed the key at it. `.add-row button` is a class and a
  # container rather than a label, per the fixture rule.
  */ai/providers|*/ai/providers?*)
    [ "$#" -eq 0 ] && set -- '.add-row button' ;;
esac

if [ "$#" -eq 0 ]; then
  set -- \
    '[data-applet-id=quick-settings]' \
    '[data-applet-id=notifications]' \
    '[data-applet-id=network]' \
    '[data-applet-id=bluetooth]' \
    '[data-applet-id=layout]' \
    '[data-applet-id=undo]' \
    '[data-applet-id=battery]@@shell-battery-panel' \
    '[data-applet-id=audio]@@shell-audio-panel' \
    '[data-applet-id=tray]@@shell-tray-panel' \
    '.mpris-art@@shell-mpris-panel'
fi

fail=0
shot=$(mktemp /tmp/arlen-escape-XXXXXX.png)
trap 'rm -f "$shot"' EXIT

# THE CONTROL FIRST, for the reason every probe control in this tree exists: a
# probe that answers nothing is indistinguishable from a probe that is not
# running, and this one answers a three-part sentence rather than a list, so a
# broken run would read as "nothing was open" on every row - a clean-looking
# sweep over a page nobody touched.
control="$("$here/headless.sh" --url "file://$here/escape-dismisses-control.html" \
  --out "$shot" --probe-file "$here/escape-dismisses.js" 2>/dev/null \
  | grep -E '^\[' | tail -n 1)"
case "$control" in
  *"before=["*"]"*"after=["*) ;;
  *)
    echo "sweep-escape.sh: escape-dismisses.js does not answer over its own control;" >&2
    echo "  every row below would read as nothing-was-open. it said: $control" >&2
    exit 2
    ;;
esac

for spec in "$@"; do
  sel="${spec%%@@*}"
  host=""
  case "$spec" in *@@*) host="${spec##*@@}" ;; esac
  hostargs=()
  if [ -n "$host" ]; then
    hostfile="$here/hosts/$host.js"
    if [ ! -f "$hostfile" ]; then
      echo "  FAIL $spec names no host: $hostfile is not there"
      fail=1
      continue
    fi
    hostargs=(--host-script "$hostfile")
  fi
  got="$("$here/headless.sh" --url "$base" --out "$shot" --open "$sel" \
    "${hostargs[@]}" --probe-file "$here/escape-dismisses.js" 2>/dev/null \
    | grep -E '^\[' | tail -n 1)"
  case "$got" in
    "["*"]") ;;
    *)
      echo "  FAIL $spec did not answer: ${got:-the probe printed nothing}"
      fail=1
      continue
      ;;
  esac
  before="$(printf '%s' "$got" | sed -n "s/.*before=\[\([^]]*\)\].*/\1/p")"
  after="$(printf '%s' "$got" | sed -n "s/.*after=\[\([^]]*\)\].*/\1/p")"
  # A MODAL THAT DOES NOT HOLD THE KEYBOARD, which is the second failure this
  # sweep can name and the one that was found by adding a reading rather than by
  # pressing a key. `aria-modal="true"` tells a screen reader the rest of the
  # page is not there; focus sitting on `body` while that is on screen leaves the
  # reader's cursor in the region it was just told to ignore, and the next Tab
  # starts at the top of the document behind the dialog. Two of the three
  # Settings dialogs answered exactly that on 11 September.
  #
  # ONLY FOR A MODAL. A popover claims nothing about the rest of the page, and
  # leaving focus where it was is the ordinary behaviour for one, so the rule
  # would be wrong for the ten shell rows. NOT evaluated over the control page:
  # its planted dialogs are static markup with no focus management at all, and
  # that page's job is to prove the probe answers, not to model a correct one.
  modal="$(printf '%s' "$got" | sed -n "s/.*modal=\([a-z]*\).*/\1/p")"
  focus_before="$(printf '%s' "$got" | sed -n "s/.*focusBefore=\([^\"]*\).*/\1/p")"
  if [ -z "$before" ]; then
    echo "  opened none  $spec"
    echo "       the click landed and no overlay was on screen, so this row proves nothing"
    fail=1
  elif [ -n "$after" ]; then
    echo "  still open   $spec"
    echo "       after Escape: $after"
    fail=1
  elif [ "$modal" = "true" ] && [ "$focus_before" = "body" ]; then
    echo "  no keyboard  $spec"
    echo "       a modal was open and focus was still on body, so a reader tabbing"
    echo "       from there starts at the top of the page behind it"
    fail=1
  else
    echo "  ok   $spec"
  fi
done

echo "--   $# overlay(s) asked whether Escape closes them; anything not named here was not looked at"
exit "$fail"
