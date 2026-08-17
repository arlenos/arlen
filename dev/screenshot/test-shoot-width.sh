#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Control for the one thing `shoot.sh` must never do again: hand back a shot
# narrower than the caller asked for and say nothing.
#
# The bug it guards was live for a week. `xvfb-run -a` with no server args gives
# a 640x480 screen, so every request for 1280 came back clamped, `--require-width`
# was opt-in and nobody passed it, and the PNGs went into the record as if they
# were desktop renders. Both halves are checked here, because fixing the screen
# without arming the refusal would leave the next screen-size regression just as
# quiet.
#
# Run: dev/screenshot/test-shoot-width.sh
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fail=0

check() {
  local name="$1" ok="$2" detail="${3:-}"
  if [ "$ok" = 1 ]; then
    echo "  ok   $name"
  else
    echo "  FAIL $name"
    [ -n "$detail" ] && echo "$detail" | sed 's/^/       /'
    fail=1
  fi
}

echo "shoot.sh width:"

# 1. A desktop request is actually served at that width. This is the fix.
out=$("$here/shoot.sh" 'data:text/html,<p>x</p>' /tmp/shoot-width-a.png 2>&1)
rc=$?
w=$(printf '%s\n' "$out" | sed -n 's/^viewport: \([0-9]*\)x.*/\1/p')
check "a 1280 request renders at 1280" \
  "$([ "$rc" = 0 ] && [ "${w:-0}" -ge 1280 ] && echo 1 || echo 0)" \
  "exit=$rc width=${w:-none}
$out"

# 2. A narrow request is honoured too, so the floor is not a minimum width rule -
#    a caller checking the phone layout must still be able to.
out=$("$here/shoot.sh" 'data:text/html,<p>x</p>' /tmp/shoot-width-b.png "" 500 400 2>&1)
rc=$?
w=$(printf '%s\n' "$out" | sed -n 's/^viewport: \([0-9]*\)x.*/\1/p')
check "a deliberate 500px request is served and accepted" \
  "$([ "$rc" = 0 ] && [ "${w:-0}" = 500 ] && echo 1 || echo 0)" \
  "exit=$rc width=${w:-none}
$out"

# 3. Put the bug back: a screen too small for the request must be refused, and no
#    PNG left behind for the next reader to mistake for a result.
rm -f /tmp/shoot-width-c.png
out=$(SHOOT_SCREEN_W=640 SHOOT_SCREEN_H=480 \
      xvfb-run -a --server-args="-screen 0 640x480x24" bash -c '
  unset WAYLAND_DISPLAY; export GDK_BACKEND=x11
  WebKitWebDriver --port=4479 >/dev/null 2>&1 & wd=$!
  trap "kill $wd 2>/dev/null || true" EXIT
  for _ in $(seq 1 25); do curl -s http://localhost:4479/status >/dev/null 2>&1 && break; sleep 0.2; done
  python3 '"$here"'/shoot.py --url "data:text/html,<p>x</p>" \
    --out /tmp/shoot-width-c.png --port 4479 --width 1280 --height 800' 2>&1)
rc=$?
check "a screen too small for the request is refused" \
  "$([ "$rc" != 0 ] && [ ! -e /tmp/shoot-width-c.png ] && echo 1 || echo 0)" \
  "exit=$rc png=$([ -e /tmp/shoot-width-c.png ] && echo written || echo absent)
$out"

rm -f /tmp/shoot-width-a.png /tmp/shoot-width-b.png /tmp/shoot-width-c.png
[ "$fail" = 0 ] && echo "a clamped viewport cannot pass for a desktop render any more"
exit "$fail"
