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
#
# The second form runs with WEBKIT_FORCE_SANDBOX=0 and must report NOT contained.
# A probe that says "contained" either way proves nothing, so run both before
# believing the first.
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
APP_DIR=$(printf '%s' "$BIN" | sed 's#.*/##; s/^arlen-//; s/-app$//')
for candidate in "apps/$APP_DIR/src-tauri/src" "apps/$APP_DIR/src"; do
  [ -d "$candidate" ] || continue
  newer=$(find "$candidate" -newer "$BIN" -name '*.rs' -o -newer "$BIN" -name '*.svelte' 2>/dev/null | head -1)
  if [ -n "$newer" ]; then
    echo "stale binary: $BIN is older than $newer" >&2
    echo "rebuild it, or the verdict describes a build that no longer exists" >&2
    exit 2
  fi
done

Xvfb "$DISPLAY_NUM" -screen 0 1280x800x24 >/tmp/probe-xvfb.log 2>&1 &
XVFB=$!
sleep 2

if [ "$MODE" = "off" ]; then
  env DISPLAY="$DISPLAY_NUM" WEBKIT_FORCE_SANDBOX=0 "$BIN" >/tmp/probe-app.log 2>&1 &
else
  env DISPLAY="$DISPLAY_NUM" "$BIN" >/tmp/probe-app.log 2>&1 &
fi
APP=$!
sleep "$SETTLE"

# The report is piped through `tee`, which runs it in a subshell, so the verdict
# cannot come back in a variable - it goes through a file. Found by reading the
# exit code rather than the text: the first version printed CONTAINED and exited
# 2 every time, which is a probe nothing could gate on.
VERDICT_FILE=$(mktemp)
echo 2 >"$VERDICT_FILE"
{
  echo "probe: $BIN (sandbox requested: $MODE)"

  # The web process under bwrap is a grandchild, so find it by name rather than
  # by walking the tree, and take the one whose parent is a bwrap.
  web=""
  for p in $(pgrep -f "WebKitWebProcess" 2>/dev/null); do
    parent=$(ps -o comm= -p "$(ps -o ppid= -p "$p" 2>/dev/null | tr -d ' ')" 2>/dev/null)
    [ "$parent" = "bwrap" ] && web="$p" && break
    [ -z "$web" ] && web="$p"
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
kill "$XVFB" 2>/dev/null
wait 2>/dev/null
verdict=$(cat "$VERDICT_FILE")
rm -f "$VERDICT_FILE"
exit "$verdict"
