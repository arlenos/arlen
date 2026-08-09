#!/bin/bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Watch an app's WebKit web process and say whether it is actually contained.
#
# `check-webview-sandbox.py` reads source and can only say that every app ASKS
# for containment. This runs one and looks at what the kernel did, which is the
# difference between the posture we describe and the posture we have.
#
# What it looks at: WebKit launches its web process through `bwrap` when the
# sandbox is on, so the process ends up in its own mount, user and pid
# namespaces. Comparing those against the app's own is a fact about the running
# system, not an inference from a flag.
#
# Usage:  dev/scripts/probe-webview-sandbox.sh target/debug/arlen-system-monitor
#         dev/scripts/probe-webview-sandbox.sh <binary> off    # negative control
#         PROBE_WAYLAND=1 dev/scripts/probe-webview-sandbox.sh target/debug/arlen-desktop-shell
#
# The second form disables the sandbox and must report NOT contained. A probe
# that says "contained" either way proves nothing, so run both before believing
# the first. NB the variable changed: WebKitGTK 2.52 says so itself, in the
# library - "WEBKIT_FORCE_SANDBOX no longer allows disabling the sandbox. Use
# WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1 instead." The old name left the
# negative control silently passing, which is the one failure a control must not
# have.
#
# The third form is for the desktop shell, which is a wlr-layer-shell client and
# cannot come up on a bare X server - under Xvfb it reports "no web process
# appeared", which is inconclusive and reads like a pass to anyone skimming. It
# boots the real compositor nested through `dev/screenshot/shoot-compositor.sh`
# and hosts the app under that, so the same comparison happens against a shell
# that actually started. Slower, and it needs the compositor repo built.
#
# Not in CI: it needs a built Tauri binary, an X server and about ten seconds.

set -u

BIN="${1:?usage: probe-webview-sandbox.sh <app-binary> [off]}"
MODE="${2:-on}"
DISPLAY_NUM="${PROBE_DISPLAY:-:77}"
SETTLE="${PROBE_SETTLE:-8}"

if [ ! -x "$BIN" ]; then
  echo "no such binary: $BIN (build it first)" >&2
  exit 2
fi
command -v Xvfb >/dev/null || { echo "Xvfb is not installed" >&2; exit 2; }

# This probe measures a BINARY, and a binary is a claim about the past. The
# knowledge app reported NOT CONTAINED twice and looked like a real hole in an
# app that renders filenames from disk; it was a binary built the day before the
# line that asks for containment was written, and the string is simply not in it.
# A verification that silently measures history is worse than no verification, so
# an app whose source is newer than its binary stops here rather than producing a
# verdict about a build nobody is shipping.
#
# Only the Rust side is compared. What decides this verdict is one line in
# `main.rs`, and the frontend cannot affect it; refusing because a stylesheet
# moved would be a guard people learn to skip.
APP_DIR=$(printf '%s' "$BIN" | sed 's#.*/##; s/^arlen-//; s/-app$//')
HOST="apps/$APP_DIR/src-tauri/src"
if [ -d "$HOST" ]; then
  newer=$(find "$HOST" -name '*.rs' -newer "$BIN" 2>/dev/null | head -1)
  if [ -n "$newer" ]; then
    echo "stale binary: $BIN is older than $newer" >&2
    echo "rebuild it, or the verdict describes a build that no longer exists" >&2
    exit 2
  fi
fi

WAYLAND="${PROBE_WAYLAND:-0}"

if [ "$WAYLAND" = "1" ]; then
  # The compositor harness brings its own Xvfb and its own settle, and it hosts
  # the app as a Wayland client. We are one of its two consumers, so it runs in
  # the background and we look at the tree while the client is alive.
  # The client inherits this process's environment, so the negative control still
  # reaches it. Without this line `off` would be accepted and quietly ignored on
  # this path, and a probe that cannot fail is not evidence of anything.
  [ "$MODE" = "off" ] && export WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1
  SHOOT_SETTLE=$((SETTLE + 6)) SHOOT_CLIENT_LOG=/tmp/probe-app.log \
    bash "$(dirname "$0")/../screenshot/shoot-compositor.sh" \
      /tmp/probe-webview-sandbox.png "$BIN" >/tmp/probe-compositor.log 2>&1 &
  HARNESS=$!
  XVFB=""
  sleep "$((SETTLE + 8))"
else
  Xvfb "$DISPLAY_NUM" -screen 0 1280x800x24 >/tmp/probe-xvfb.log 2>&1 &
  XVFB=$!
  sleep 2

  if [ "$MODE" = "off" ]; then
    env DISPLAY="$DISPLAY_NUM" WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1 "$BIN" >/tmp/probe-app.log 2>&1 &
  else
    env DISPLAY="$DISPLAY_NUM" "$BIN" >/tmp/probe-app.log 2>&1 &
  fi
  APP=$!
  HARNESS=""
  sleep "$SETTLE"
fi

# Under the compositor the app is the harness's grandchild, not our child, so the
# pid has to be looked up. `comm` is truncated to 15 characters by the kernel,
# which is why this matches a prefix rather than the whole name - a lookup by
# full command line matches this script's own arguments instead, and reports the
# probe's namespaces as the app's.
if [ "$WAYLAND" = "1" ]; then
  want=$(printf '%s' "${BIN##*/}" | cut -c1-15)
  APP=""
  for d in /proc/[0-9]*; do
    [ "$(cat "$d/comm" 2>/dev/null)" = "$want" ] && APP="${d#/proc/}" && break
  done
  APP="${APP:-0}"
fi

# The report is piped through `tee`, which runs it in a subshell, so the verdict
# cannot come back in a variable - it goes through a file. Found by reading the
# exit code rather than the text: the first version printed CONTAINED and exited
# 2 every time, which is a probe nothing could gate on.
VERDICT_FILE=$(mktemp)
echo 2 >"$VERDICT_FILE"
{
  echo "probe: $BIN (sandbox requested: $MODE)"

  # The web process must be a DESCENDANT of the app under test. Selecting it by
  # name across the machine is how this came to compare an app started seconds
  # ago against a five-hour-old web process belonging to a different app, and
  # report CONTAINED on the strength of it - on 9 August, ten times in a row,
  # because a probing session leaves those behind. `pgrep -f` also matches any
  # command line mentioning the name, including this script's own shell, so the
  # match is by comm and then by ancestry.
  # `WebKitWebProcess` is 16 characters and the kernel truncates `comm` to 15, so
  # `pgrep -x WebKitWebProcess` matches nothing at all - pgrep says so and exits,
  # which reads as "no web process appeared" rather than as a broken pattern. The
  # same truncation is already handled for the app lookup above.
  web=""
  for d in /proc/[0-9]*; do
    case "$(cat "$d/comm" 2>/dev/null)" in
      WebKitWebProces*) p="${d#/proc/}" ;;
      *) continue ;;
    esac
    walk="$p"
    while [ -n "$walk" ] && [ "$walk" != "1" ] && [ "$walk" != "0" ]; do
      walk=$(ps -o ppid= -p "$walk" 2>/dev/null | tr -d ' ')
      if [ "$walk" = "$APP" ]; then
        web="$p"
        break
      fi
    done
    [ -n "$web" ] && break
  done

  if [ -z "$web" ]; then
    echo "  no WebKit web process appeared - the app may not have started."
    echo "  This is INCONCLUSIVE, not a pass: see /tmp/probe-app.log"
    echo 2 >"$VERDICT_FILE"
  else
    app_mnt=$(readlink "/proc/$APP/ns/mnt" 2>/dev/null)
    web_mnt=$(readlink "/proc/$web/ns/mnt" 2>/dev/null)
    web_usr=$(readlink "/proc/$web/ns/user" 2>/dev/null)
    app_usr=$(readlink "/proc/$APP/ns/user" 2>/dev/null)
    echo "  app pid $APP  mnt=$app_mnt user=$app_usr"
    echo "  web pid $web  mnt=$web_mnt user=$web_usr"
    if [ -n "$web_mnt" ] && [ "$web_mnt" != "$app_mnt" ] && [ "$web_usr" != "$app_usr" ]; then
      echo "  CONTAINED: the web process has its own mount and user namespace."
      echo 0 >"$VERDICT_FILE"
    else
      echo "  NOT CONTAINED: the web process shares the app's namespaces, so it"
      echo "  can reach whatever the app can."
      echo 1 >"$VERDICT_FILE"
    fi
  fi
  echo "  (a proxied bus is part of the same containment; look for xdg-dbus-proxy"
  echo "   under bwrap in the tree if you want to see it)"
} | tee /tmp/probe-webview-sandbox.out

kill "$APP" 2>/dev/null
sleep 1
[ -n "$XVFB" ] && kill "$XVFB" 2>/dev/null
# The compositor harness tears down its own compositor and Xvfb when it finishes
# its capture; killing it mid-run would leave those behind, so it is waited out.
[ -n "$HARNESS" ] && wait "$HARNESS" 2>/dev/null
wait 2>/dev/null
verdict=$(cat "$VERDICT_FILE")
rm -f "$VERDICT_FILE"
exit "$verdict"
