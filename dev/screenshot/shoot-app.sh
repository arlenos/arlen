#!/usr/bin/env bash
# Test Layer 1b (full app): launch a REAL Tauri binary via tauri-driver under
# Xvfb and screenshot it. Unlike shoot.sh (a webview URL in isolation), this runs
# the actual app - Rust backend + webview together - so it verifies the whole
# thing (IPC + render), e.g. that terminal command output appears.
#
# Usage:
#   dev/screenshot/shoot-app.sh <app-binary> <out.png> [type-text]
#
#   <app-binary>  a built Tauri binary that serves its frontend from frontendDist
#                 (run the app's `npm run build` then `cargo build` first)
#   <out.png>     where to write the PNG
#   [type-text]   optional text typed into the focused input then submitted with
#                 Enter (e.g. a terminal command), so output renders before the shot
#
# Requirements: tauri-driver (cargo install tauri-driver), WebKitWebDriver, Xvfb.
#
# A DEBUG binary loads `build.devUrl` from tauri.conf.json, not the bundled
# frontend, so `npm run build` + `cargo build` alone is not enough: with nothing
# on that port the webview renders its own "Connection refused" page and the shot
# is worthless. Either `cargo build --release`, or serve the built frontend on the
# port the app's tauri.conf.json names:
#
#   ( cd apps/<app> && npx vite preview --port <devUrl port> --outDir build & )
#
# Since 9 August this is checked rather than left to the eye - the driver reads
# the DOM after the shot and exits 1 on an error page.
set -euo pipefail

# Usage: shoot-app.sh <app-binary> [out.png] [type-text] [settle]
# Screenshot mode needs <out.png>. Assert mode (SHOOT_EXEC set) runs a command in
# the terminal and asserts SHOOT_EXPECT renders, with no screenshot - leave out.png
# empty (e.g. `SHOOT_EXEC='echo hi' SHOOT_EXPECT=hi shoot-app.sh <bin>`).
export SHOOT_APP="${1:?usage: shoot-app.sh <app-binary> [out.png] [type-text] [settle]}"
export SHOOT_OUT="${2:-}"
export SHOOT_TYPE="${3:-}"
# Seconds to wait for the app to come up and hydrate before querying the DOM or
# screenshotting. A heavy SvelteKit app under WebKitGTK + Xvfb needs more than the
# 3s default, or `.console` is not mounted yet and the shot races the paint.
export SHOOT_SETTLE="${4:-}"
# A JS file to run in the page, its return value printed as `inject result: ...`.
# The vite-served scan cannot see anything a Tauri command supplies, because there
# is no Tauri there; this harness runs the real binary, so it can.
export SHOOT_INJECT="${SHOOT_INJECT:-}"
export SHOOT_LOCALE="${SHOOT_LOCALE:-}"
# SHOOT_APP_ENV="NAME=value;OTHER=value" runs the app with those variables set.
#
# Semicolon-separated, not colon, because the variable most worth setting is PATH
# and its own value is full of colons.
#
# WHY A WRAPPER AND NOT THE HARNESS ENVIRONMENT. The app inherits whatever
# tauri-driver has, so exporting here would change the tooling too - and for PATH
# that means `timeout`, `curl` and the driver itself disappear, which is exactly
# how the first attempt at this failed. A generated script that sets the variables
# and `exec`s the real binary keeps the change to the app alone, and `exec` means
# the pid the driver is managing is still the app.
export SHOOT_APP_ENV="${SHOOT_APP_ENV:-}"
# A file to launch the app on, colon-separated for more than one argument.
export SHOOT_APP_ARGS="${SHOOT_APP_ARGS:-}"
# The binary is an argument here, so building it is the caller's job - but its age
# is not their memory. On 6 August the compositor harness screenshotted a binary
# six weeks old and reported a pass, so any harness that runs a prebuilt artifact
# says how old it is.
#
# THE DATE ALONE IS NOT ENOUGH, measured again on 28 August: the Windows-panel
# drive ran a settings binary from four days earlier and reported a banner missing
# that had been fixed since, and the date was printed and read and still cost a
# cycle. A date asks the reader to remember when they last built; "older than its
# source" is the sentence they actually need.
#
# The crate is found by NAME rather than from a table: whichever `Cargo.toml`
# declares this binary owns the sources compared against it, so a new app needs no
# entry here. Not fatal - running an old binary on purpose is a real thing to do -
# but it says so where it cannot be missed.
#
# It compares MTIMES, so a `git checkout`, a branch switch or a stash pop makes it
# fire without anybody editing anything. That direction is the cheap one: a false
# warning costs a rebuild, a missing one costs the cycle described above.
if [ -e "$SHOOT_APP" ]; then
  echo "app binary: $SHOOT_APP (built $(date -r "$SHOOT_APP" '+%Y-%m-%d %H:%M'))" >&2
  _repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
  _bin="$(basename "$SHOOT_APP")"
  _crate="$(grep -rl "^name = \"$_bin\"" "$_repo"/apps/*/src-tauri/Cargo.toml 2>/dev/null | head -1)"
  if [ -n "$_crate" ]; then
    # The first version of this was a `find | while read` pipeline, and a pipeline
    # whose loop ends without matching returns non-zero: under `set -e` the
    # assignment carrying it killed the script, so every drive printed one line and
    # then reported assertions about a page it never loaded. `find -print -quit`
    # cannot fail that way - matching nothing is a 0 exit - and `|| true` covers
    # what is left, an unreadable directory. The parentheses matter too: without
    # them the `-o` binds so that `-newer` applies to the first name only.
    _newer="$(find "$(dirname "$_crate")/src" "$(dirname "$(dirname "$_crate")")/src" \
                \( -name '*.rs' -o -name '*.svelte' -o -name '*.ts' \) \
                -newer "$SHOOT_APP" -print -quit 2>/dev/null || true)"
    if [ -n "$_newer" ]; then
      echo "!! that binary is OLDER than its source ($_newer changed since it was" >&2
      echo "   built). Whatever this run reports is about the old code - rebuild" >&2
      echo "   before believing a failure." >&2
    fi
  fi
fi

export SHOOT_PORT=4444
export SHOOT_HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export NATIVE="$(command -v WebKitWebDriver || echo /usr/bin/WebKitWebDriver)"

# tauri-driver spawns the app + the native WebKitWebDriver; the python client
# talks to tauri-driver. All inputs travel as env (not interpolated) so a typed
# command with spaces/quotes is safe.
# Xvfb is the one non-obvious dependency of the whole screenshot loop, and
# without it these scripts died with a bare "xvfb-run: command not found" that
# says nothing about what to install. The screenshot loop is mandatory for any
# UI change, so failing to run it must be actionable, never cryptic.
require_xvfb() {
  local bin="$1"
  command -v "$bin" >/dev/null 2>&1 && return 0
  echo "error: '$bin' not found - the headless screenshot loop needs Xvfb." >&2
  echo "  Arch/EndeavourOS: sudo pacman -S xorg-server-xvfb" >&2
  echo "  Debian/Ubuntu:    sudo apt install xvfb" >&2
  echo "  Fedora:           sudo dnf install xorg-x11-server-Xvfb" >&2
  exit 127
}
require_xvfb xvfb-run

# THE HOST SESSION IS CUT OFF HERE, and it was not before 15 August.
#
# `xvfb-run` sets DISPLAY and touches nothing else, so WAYLAND_DISPLAY still pointed
# at the developer's real compositor for everything launched below. That is not a
# theoretical leak: rendering the screenshot app under this harness connected it to
# the live session over wlr-screencopy and wrote a picture of the developer's actual
# desktop - terminal contents, browser tabs, chat - into /tmp, in a run whose whole
# premise was that it is headless.
#
# It also quietly undermines every other shot: an app that prefers the Wayland
# backend was talking to the real compositor, so "verified headless" meant less than
# it read. GDK_BACKEND=x11 makes the toolkit choice explicit rather than leaving it
# to whichever socket happens to be reachable.
#
# The compositor harness (shoot-compositor.sh) is the place where a real Wayland
# session is the point; this one is Xvfb, and an app here has no business reaching
# outside it.
xvfb-run -a --server-args="-screen 0 1280x900x24" bash -c '
  set -euo pipefail
  unset WAYLAND_DISPLAY
  export GDK_BACKEND=x11
  # A window manager so the WebKit app window holds real keyboard focus; without
  # one, synthetic keystrokes never route to the focusable .console surface (so
  # the assert mode cannot drive the terminal). Best-effort: harmless if absent.
  ob=""
  if command -v openbox >/dev/null 2>&1; then openbox >/tmp/arlen-openbox.log 2>&1 & ob=$!; sleep 1.5; fi
  # Refuse to start on top of a driver that is already there.
  #
  # The port is fixed, and the readiness check below is "does :4444 answer" -
  # which the driver from a PREVIOUS run answers perfectly well while it is
  # shutting down. Back-to-back shots are the normal way this script is used, so
  # without this the second one can attach to the driver of the first, drive a
  # window that is going away, and report whatever it managed to capture as a
  # result about the app under test.
  #
  # No apostrophes in this block, comments included: it is the body of a
  # single-quoted bash -c, and the outer shell ends that string at the first one
  # it meets, comment or not. Writing "run" + "s" cost me a parse error here.
  if curl -s "http://localhost:$SHOOT_PORT/status" >/dev/null 2>&1; then
    # No single quotes anywhere in here: this whole block is the body of a
    # single-quoted bash -c, so one would close it and the script would die at
    # parse time with an unmatched-quote error pointing at the wrong line.
    echo "a webdriver is already listening on port $SHOOT_PORT: refusing to run" >&2
    echo "rather than attach to it. Wait for the previous shot to finish, or kill" >&2
    echo "the leftover with: pkill -f tauri-driver" >&2
    exit 1
  fi
  tauri-driver --port "$SHOOT_PORT" --native-driver "$NATIVE" \
    >/tmp/arlen-tauri-driver.log 2>&1 &
  td=$!
  # Kill AND wait. A trap that only signals returns while the driver is still
  # holding the port, so the next run races the corpse of this one - which is how
  # the check above would otherwise get to fire on a healthy machine.
  trap "kill $td ${ob} 2>/dev/null || true; wait $td ${ob} 2>/dev/null || true" EXIT
  for _ in $(seq 1 50); do
    curl -s "http://localhost:$SHOOT_PORT/status" >/dev/null 2>&1 && break
    sleep 0.2
  done
  app_under_test="$SHOOT_APP"
  if [ -n "${SHOOT_APP_ENV:-}" ]; then
    wrapper="$(mktemp -d)/app-under-test"
    {
      echo "#!/bin/sh"
      IFS=";" read -ra pairs <<< "$SHOOT_APP_ENV"
      for pair in "${pairs[@]}"; do
        [ -n "$pair" ] || continue
        echo "export ${pair%%=*}=\"${pair#*=}\""
      done
      echo "exec \"$SHOOT_APP\" \"\$@\""
    } > "$wrapper"
    chmod +x "$wrapper"
    app_under_test="$wrapper"
    echo "app runs with: $SHOOT_APP_ENV"
  fi
  args=(--app "$app_under_test" --port "$SHOOT_PORT")
  [ -n "$SHOOT_OUT" ] && args+=(--out "$SHOOT_OUT")
  [ -n "$SHOOT_TYPE" ] && args+=(--type "$SHOOT_TYPE")
  [ -n "$SHOOT_SETTLE" ] && args+=(--settle "$SHOOT_SETTLE")
  [ -n "${SHOOT_GRAB:-}" ] && args+=(--grab-x)
  # SHOOT_LOCALE=de renders the translated half, which is the half no English
  # shot can check.
  [ -n "${SHOOT_LOCALE:-}" ] && args+=(--locale "$SHOOT_LOCALE")
  # Colon-separated, so an app can be launched on a file: SHOOT_APP_ARGS=/etc/hosts
  if [ -n "${SHOOT_APP_ARGS:-}" ]; then
    IFS=":" read -ra appargs <<< "$SHOOT_APP_ARGS"
    for a in "${appargs[@]}"; do args+=(--app-arg "$a"); done
  fi
  [ -n "${SHOOT_EXEC:-}" ] && args+=(--exec "$SHOOT_EXEC")
  [ -n "${SHOOT_EXPECT:-}" ] && args+=(--expect "$SHOOT_EXPECT")
  # Colon-separated, so a caller can move the app then ask about where it went.
  if [ -n "${SHOOT_INJECT:-}" ]; then
    IFS=':' read -ra injects <<< "$SHOOT_INJECT"
    for f in "${injects[@]}"; do args+=(--inject "$f"); done
  fi
  [ -n "${SHOOT_INJECT_SETTLE:-}" ] && args+=(--inject-settle "$SHOOT_INJECT_SETTLE")
  python3 "$SHOOT_HERE/shoot_app.py" "${args[@]}"
'

# THE SHOT HAS TO EXIST. This script spent an hour on 28 August exiting 0 having
# done nothing at all - a `find | while read` pipeline above returned non-zero, the
# assignment carrying it failed under `set -e`, and everything below never ran. The drives then reported
# assertions about a page that was never loaded, which reads exactly like a broken
# app. Whatever goes wrong above, "no file was written" is not a success.
if [ ! -s "$SHOOT_OUT" ]; then
  echo "!! no shot was written to $SHOOT_OUT - this run proved nothing about the app" >&2
  exit 1
fi
