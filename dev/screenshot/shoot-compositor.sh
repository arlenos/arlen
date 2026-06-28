#!/usr/bin/env bash
# Headless full-app screenshot under the REAL Arlen compositor. Runs cosmic-comp
# nested in Xvfb (its winit X11 backend), launches a Wayland client under it, and
# grim-captures the composited output.
#
# Unlike shoot-app.sh (X11 / WebKitWebDriver, no Wayland) this hosts genuine
# Wayland clients - the shell's wlr-layer-shell topbar, the Tauri apps - so it is
# the path for verifying shell + compositor UI that the webview-only harness
# cannot reach (the top bar, window decorations, cross-app focus).
#
# Usage:
#   dev/screenshot/shoot-compositor.sh <out.png> <client-cmd> [args...]
#
#   <out.png>     where to write the screenshot
#   <client-cmd>  the Wayland client to launch; it is run with WAYLAND_DISPLAY set
#                 to the compositor's socket and DISPLAY cleared
#
# This is the closed nested verify loop (autonomous-verify-pipeline-plan.md): boot
# the compositor nested -> optionally INJECT input -> grim-capture -> optionally
# COMPARE to a baseline. With no inject/baseline it is just a capture (its original
# use). With them it is a self-checking regression tripwire.
#
# Env:
#   COMPOSITOR_PATH    the compositor repo (default ~/Repositories/compositor)
#   SHOOT_SETTLE       seconds to wait for the client to render (default 5)
#   SHOOT_DISPLAY      the Xvfb display to use (default :99)
#   SHOOT_CLIENT_LOG   capture the client's stdout/stderr here (default /dev/null);
#                      set it to a file to debug why a client did not render
#   SHOOT_INJECT       a command run after settle, before capture, to inject input.
#                      NOTE on reach (tested 28 Jun): injecting into the NESTED
#                      surface under this Xvfb harness is unsolved. The compositor
#                      runs its x11 backend (picks x11 when DISPLAY is set) reading
#                      XInput2 events on its Xvfb window, but `xdotool`/XTEST into
#                      Xvfb did NOT reach the nested Wayland client even with
#                      windowfocus (the key never appeared at a nested shell prompt);
#                      ydotool/uinput inject at the evdev layer and reach the HOST
#                      seat, not the nested surface. So this harness is reliable for
#                      CAPTURE/render verification (grim of compositor chrome + the
#                      client surface), and inject-requiring tests (click-path) need
#                      the QEMU VM pass (QMP input-send-event into a real-evdev guest)
#                      or a DRM/headless-seat nested setup, not this Xvfb path. The
#                      command still runs with both the Xvfb DISPLAY and the
#                      compositor's WAYLAND_DISPLAY set (so an X11 tool can at least
#                      connect to Xvfb, e.g. to inspect windows).
#   SHOOT_INJECT_SETTLE seconds to wait after inject before capture (default 1)
#   SHOOT_CLIENT2      a second client command launched under the compositor after
#                      the first settles, for multi-window / tiling-chrome captures
#   SHOOT_BASELINE     a reference PNG; if set, compare the capture to it after
#                      grim and FAIL (exit 3) when the differing-pixel count
#                      exceeds SHOOT_TOLERANCE. A missing baseline writes the shot
#                      and passes (first-time inspection). Net-new surfaces with no
#                      baseline are left for visual inspection of <out.png>.
#   SHOOT_TOLERANCE    max differing-pixel count for a baseline PASS (default 100)
#
# Requirements: Xvfb, grim, a built cosmic-comp at
# $COMPOSITOR_PATH/target/debug/cosmic-comp; plus ydotool/wtype if SHOOT_INJECT is
# used and imagemagick (`magick compare`) if SHOOT_BASELINE is used.
set -euo pipefail

OUT="${1:?usage: shoot-compositor.sh <out.png> <client-cmd> [args...]}"
shift
[ "$#" -ge 1 ] || { echo "usage: shoot-compositor.sh <out.png> <client-cmd> [args...]" >&2; exit 2; }

COMPOSITOR_PATH="${COMPOSITOR_PATH:-$HOME/Repositories/compositor}"
CC_BIN="$COMPOSITOR_PATH/target/debug/cosmic-comp"
[ -x "$CC_BIN" ] || { echo "no cosmic-comp at $CC_BIN (build it, or set COMPOSITOR_PATH)" >&2; exit 1; }

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
SETTLE="${SHOOT_SETTLE:-5}"
DISP="${SHOOT_DISPLAY:-:99}"
LOG="$(mktemp)"

cleanup() {
  kill "${CLIENT2_PID:-}" "${CLIENT_PID:-}" "${CC_PID:-}" "${XVFB_PID:-}" 2>/dev/null || true
  wait 2>/dev/null || true
  rm -f "/tmp/.X${DISP#:}-lock" "$LOG" 2>/dev/null || true
}
trap cleanup EXIT

rm -f "/tmp/.X${DISP#:}-lock"
Xvfb "$DISP" -screen 0 1920x1080x24 >/dev/null 2>&1 &
XVFB_PID=$!
sleep 2

DISPLAY="$DISP" "$CC_BIN" >"$LOG" 2>&1 &
CC_PID=$!

# cosmic-comp picks its own socket name (wayland-N, ignoring WAYLAND_DISPLAY); it
# logs "Listening on \"wayland-N\"". Parse it rather than guess, then wait for the
# socket file to actually exist.
WL=""
for _ in $(seq 1 40); do
  WL="$(grep -oE 'wayland-[0-9]+' "$LOG" | head -1 || true)"
  [ -n "$WL" ] && [ -S "$XDG_RUNTIME_DIR/$WL" ] && break
  sleep 0.5
done
if [ -z "$WL" ] || [ ! -S "$XDG_RUNTIME_DIR/$WL" ]; then
  echo "cosmic-comp did not come up on $DISP; last log lines:" >&2
  tail -20 "$LOG" >&2
  exit 1
fi
echo "compositor up on $WL (display $DISP)"

CLIENT_LOG="${SHOOT_CLIENT_LOG:-/dev/null}"
WAYLAND_DISPLAY="$WL" DISPLAY="" "$@" >"$CLIENT_LOG" 2>&1 &
CLIENT_PID=$!
sleep "$SETTLE"

# Optional second client (SHOOT_CLIENT2), for multi-window / tiling-chrome
# captures: a verbatim command launched under the compositor's WAYLAND_DISPLAY
# with its own settle so both surfaces paint before capture. Tiling layout (so the
# two windows sit side by side rather than stacked) is the compositor's own config
# concern, not set here.
if [ -n "${SHOOT_CLIENT2:-}" ]; then
  echo "client2: $SHOOT_CLIENT2"
  WAYLAND_DISPLAY="$WL" DISPLAY="" bash -c "$SHOOT_CLIENT2" >>"$CLIENT_LOG" 2>&1 &
  CLIENT2_PID=$!
  sleep "$SETTLE"
fi

# Optional input injection, then a brief re-settle so the result paints before
# capture. The command runs verbatim with both the Xvfb DISPLAY and the
# compositor's WAYLAND_DISPLAY set; see the SHOOT_INJECT header note on the
# nested-surface reach caveat. A failing inject is logged but does not abort the
# capture (so the shot still records the pre-inject state for debugging).
if [ -n "${SHOOT_INJECT:-}" ]; then
  echo "inject: $SHOOT_INJECT"
  WAYLAND_DISPLAY="$WL" DISPLAY="$DISP" bash -c "$SHOOT_INJECT" \
    || echo "inject step failed (continuing to capture)" >&2
  sleep "${SHOOT_INJECT_SETTLE:-1}"
fi

WAYLAND_DISPLAY="$WL" grim "$OUT"
echo "wrote $OUT"

# Optional baseline tripwire: fail if the capture differs from a reference PNG by
# more than SHOOT_TOLERANCE pixels. `magick compare -metric AE` is the installed
# odiff equivalent; it prints the differing-pixel count to stderr and exits 0/1
# (identical/differs) or >=2 on a real error (e.g. a size mismatch), which is a
# FAIL rather than a silent pass.
if [ -n "${SHOOT_BASELINE:-}" ]; then
  if [ ! -f "$SHOOT_BASELINE" ]; then
    echo "baseline $SHOOT_BASELINE not found; wrote $OUT for first-time inspection" >&2
    exit 0
  fi
  set +e
  diff_out="$(magick compare -metric AE "$SHOOT_BASELINE" "$OUT" null: 2>&1)"
  cmp_rc=$?
  set -e
  if [ "$cmp_rc" -ge 2 ]; then
    echo "FAIL: compare error (size/format mismatch?): $diff_out" >&2
    exit 3
  fi
  diff_px="${diff_out%%[!0-9]*}"
  diff_px="${diff_px:-0}"
  tol="${SHOOT_TOLERANCE:-100}"
  echo "baseline diff: ${diff_px}px (tolerance ${tol})"
  if [ "$diff_px" -gt "$tol" ]; then
    echo "FAIL: capture differs from baseline by ${diff_px}px (> ${tol})" >&2
    exit 3
  fi
  echo "PASS: within tolerance of baseline"
fi
