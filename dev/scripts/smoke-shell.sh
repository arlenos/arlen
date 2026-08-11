#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Start the desktop shell for real and check that it came up and serves.
#
# The shell had a main-thread panic in its startup path - `spawn_launch_service`
# bound its socket with tokio's listener from Tauri's setup hook, which is outside
# any runtime, so it panicked on "there is no reactor running" every single start.
# It survived indefinitely because nothing here starts the shell: 379 unit tests
# pass, every structural check passes, and the IT-1 nightly exercises the backend
# daemons rather than the desktop. The only way to see it was to run the shell and
# read its log, which is what this does.
#
# Two assertions, both about things that were silently untrue:
#
#   1. the client log has no panic in it;
#   2. the launch socket answers a request.
#
# The second is deliberately "answers", not "answers X". Which application opens a
# text file depends on the machine, and the compositor harness points
# XDG_CONFIG_HOME at a temp directory, so the shell cannot see the user's
# mimeapps.list and `no_handler` is the correct answer here. Asserting a specific
# outcome would be asserting the developer's configuration.
#
# Not in CI: it needs the compositor repo built, an X server and about half a
# minute. Run it after touching the shell's startup.
#
# Usage:  dev/scripts/smoke-shell.sh

set -uo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN="$ROOT/target/debug/arlen-desktop-shell"
LOG=$(mktemp /tmp/arlen-shell-smoke-XXXXXX.log)
ANSWER=$(mktemp /tmp/arlen-shell-answer-XXXXXX.txt)
INJECT=$(mktemp /tmp/arlen-shell-inject-XXXXXX.sh)

# On EXIT rather than at the end, because the end is not where most runs leave:
# the binary checks below bail early, and a `rm` before `exit "$rc"` covers only
# the path that reaches it. Measured on 11 Aug - a stale-binary exit left both
# files behind, +2 per run, which is how /tmp collected 81 of them.
#
# A failing run KEEPS the log and the probe answer, because those are what the
# failure is read from, and says where they are. A passing one keeps nothing:
# nobody opens the log of a green run. `$INJECT` is scaffolding and always goes.
trap 'status=$?;
      rm -f "$INJECT";
      if [ $status -eq 0 ]; then
        rm -f "$LOG" "$ANSWER";
      else
        echo "(kept for the failure: shell log $LOG, probe answer $ANSWER)" >&2;
      fi' EXIT

[ -x "$BIN" ] || { echo "no shell binary at $BIN - cargo build it first" >&2; exit 2; }

# The binary is what runs, so a binary older than the startup code it is supposed
# to prove would make this a check on history. Same trap the sandbox probe fell
# into on the knowledge app.
newer=$(find "$ROOT/apps/desktop-shell/src-tauri/src" -name '*.rs' -newer "$BIN" 2>/dev/null | head -1)
if [ -n "$newer" ]; then
  echo "stale binary: $BIN is older than $newer - rebuild it" >&2
  exit 2
fi

cat > "$INJECT" <<EOF
#!/usr/bin/env bash
python3 "$ROOT/dev/scripts/probe-launch-socket.py" > "$ANSWER" 2>&1
EOF
chmod +x "$INJECT"

SHOOT_SETTLE=${SHOOT_SETTLE:-14} SHOOT_CLIENT_LOG="$LOG" SHOOT_INJECT="$INJECT" \
  bash "$ROOT/dev/screenshot/shoot-compositor.sh" /tmp/arlen-shell-smoke.png "$BIN" \
  >/tmp/arlen-shell-smoke-harness.log 2>&1

rc=0

if grep -q "panicked" "$LOG"; then
  echo "FAIL: the shell panicked during startup"
  grep -A2 "panicked" "$LOG" | head -6
  rc=1
else
  echo "ok: no panic in the shell's startup log"
fi

if grep -q "outcome" "$ANSWER" 2>/dev/null; then
  echo "ok: the launch socket answered"
  sed 's/^/    /' "$ANSWER"
else
  echo "FAIL: the launch socket did not answer"
  echo "  probe said:"
  sed 's/^/    /' "$ANSWER" 2>/dev/null
  echo "  shell log tail:"
  tail -5 "$LOG" | sed 's/^/    /'
  rc=1
fi


exit "$rc"
