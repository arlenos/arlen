#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Run something under an Xvfb whose display number is OURS.
#
# `xvfb-run -a` looks for a free display and then creates its lock, and those two
# steps are not atomic - so two renders starting at the same moment can choose
# the same `:N`, and what happens then is not an error: the second Xvfb loses,
# openbox and WebKit attach to a server that is going away, and BOTH runs hang
# with no output. Measured on 10 September, when a full sweep and another lane's
# walk deadlocked for half an hour and every app after the first reported "the
# probe cannot see its own control" - which reads as a broken probe and is a
# stuck display.
#
# WHY THIS FILE EXISTS RATHER THAN THE LOOP LIVING IN ONE SCRIPT. That fix landed
# in `headless.sh` and nowhere else, and a sweep renders through BOTH runners:
# `shoot.sh` takes the picture and `headless.sh` reads the page. So half of every
# sweep row went on racing, and on 11 September a knowledge sweep aborted at its
# control the moment a second render started beside it - the same symptom, from
# the copy that had not been fixed. Two copies of a careful thing drift, which is
# the sentence this whole harness keeps re-learning.
#
# Derived from the pid, the way `shoot.sh` derives its driver port and
# `sweep-render-all.sh` its app ports, then walked upward on collision, because a
# derived number is only unlikely to clash rather than certain not to.
#
#   . "$here/lib/own-display.sh"
#   own_display "-screen 0 1600x1200x24" bash -c '...'
#
# Exit code is the command's own, EXCEPT 2, which is `xvfb-run`'s "the server
# would not start" - on a taken display that is what a collision looks like, and
# it is the one code worth retrying on. A failing render must never be retried
# into a pass.
# ANOTHER X SERVER IS WORTH SAYING OUT LOUD. A display number of our own stops
# two renders taking the SAME `:N`, and it does not stop them competing for the
# one GPU underneath: with a second Xvfb alive on this machine, renders here fail
# intermittently - a flat frame, a viewport that never took the zoom, or a run
# that exits without printing anything at all, which reads as a probe that
# answered nothing.
#
# Measured on 11 September and it cost an hour: a sweep killed with `pkill` left
# an orphaned Xvfb behind, and after that a STATIC control page rendered one time
# in three. I diagnosed an app change from those numbers and was wrong - the same
# row passed eleven times running once the orphan was gone. So the note names the
# other server rather than guessing about the page.
_note_other_displays() {
  local others
  others="$(pgrep -a Xvfb 2>/dev/null | grep -v "^$$ " || true)"
  [ -n "$others" ] || return 0
  echo "note: another Xvfb is running on this machine, so a render here can fail" >&2
  echo "  intermittently - a flat frame or no answer at all. It is:" >&2
  printf '  %s\n' "$others" >&2
}

own_display() {
  local server_args="$1"
  shift
  _note_other_displays
  local base=$(( 90 + ($$ % 60) ))
  local try n rc
  # THE CALLER'S `set -e` IS THE CALLER'S. Turning errexit off to read the exit
  # code and then switching it back ON unconditionally would hand it to every
  # script that did not ask for it - `window-title.sh` and `test-shoot-width.sh`
  # both run under `set -uo pipefail` on purpose, and everything after their
  # render would suddenly start exiting on a non-zero. Saved and restored instead.
  local had_e=0
  case "$-" in *e*) had_e=1 ;; esac
  for try in 0 1 2 3 4 5 6 7; do
    n=$(( base + try ))
    if [ -e "/tmp/.X${n}-lock" ]; then
      continue
    fi
    set +e
    xvfb-run -n "$n" --server-args="$server_args" "$@"
    rc=$?
    if [ "$had_e" -eq 1 ]; then
      set -e
    fi
    if [ "$rc" -ne 2 ]; then
      return "$rc"
    fi
  done
  # Every derived number was taken. Let xvfb-run choose, which is where this
  # started: better a small race than no render at all.
  xvfb-run -a --server-args="$server_args" "$@"
}
