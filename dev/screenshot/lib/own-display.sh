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
own_display() {
  local server_args="$1"
  shift
  local base=$(( 90 + ($$ % 60) ))
  local try n rc
  for try in 0 1 2 3 4 5 6 7; do
    n=$(( base + try ))
    if [ -e "/tmp/.X${n}-lock" ]; then
      continue
    fi
    set +e
    xvfb-run -n "$n" --server-args="$server_args" "$@"
    rc=$?
    set -e
    if [ "$rc" -ne 2 ]; then
      return "$rc"
    fi
  done
  # Every derived number was taken. Let xvfb-run choose, which is where this
  # started: better a small race than no render at all.
  xvfb-run -a --server-args="$server_args" "$@"
}
