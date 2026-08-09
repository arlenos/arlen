#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Start each app and check it does not die on the way up.
#
# `smoke` does this for the daemons, and its header says exactly why: a component
# that "compiles, passes its suite and still dies on startup" is invisible to
# every check that reads files. The desktop apps had no such thing, and the
# desktop shell spent as long as the launch socket existed taking a main-thread
# panic on every start while 379 unit tests and every structural check passed.
#
# What it asserts, per app: no panic in the log, and the process is still alive
# after the settle. Both are weak assertions on purpose - an app that reaches its
# window and sits there is what this rules in, and what the window CONTAINS is the
# render harness's question (`dev/screenshot/shoot-app.sh`), not this one's.
#
# The shell is not here: it is a wlr-layer-shell client and needs the nested
# compositor, which is `just shell-smoke`.
#
# Usage:  dev/scripts/smoke-apps.sh

set -uo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
DISPLAY_NUM="${SMOKE_DISPLAY:-:81}"
SETTLE="${SMOKE_SETTLE:-8}"

# The binary each app builds. Kept as data so `check-smoke-coverage.py` can
# compare it to the tree and notice an app nobody added here.
APPS=(
    "arlen-clock-app"
    "arlen-files"
    "arlen-knowledge-app"
    "arlen-meetings"
    "arlen-screenshot"
    "arlen-settings"
    "arlen-system-monitor"
    "arlen-terminal"
    "arlen-text-editor"
    "arlen-viewers"
)

# An app that cannot start unattended, with the reason. Empty reasons are not
# accepted by the coverage check.
SKIPPED=(
    "arlen-desktop-shell|a layer-shell client: needs the nested compositor, so it has its own recipe (just shell-smoke)"
    "arlen-greeter|runs as the greeter user against greetd, before a session exists"
    "arlen-harness|arlen-ui's live work"
    "arlen-store|arlen-ui's live work"
)

command -v Xvfb >/dev/null || { echo "Xvfb is not installed" >&2; exit 2; }

Xvfb "$DISPLAY_NUM" -screen 0 1280x800x24 >/tmp/smoke-apps-xvfb.log 2>&1 &
XVFB=$!
sleep 2
trap 'kill "$XVFB" 2>/dev/null' EXIT

rc=0
for name in "${APPS[@]}"; do
    bin="$ROOT/target/debug/$name"
    if [ ! -x "$bin" ]; then
        echo "SKIP $name (not built)"
        continue
    fi
    log="/tmp/smoke-app-$name.log"
    DISPLAY="$DISPLAY_NUM" "$bin" >"$log" 2>&1 &
    pid=$!
    sleep "$SETTLE"
    if grep -q "panicked" "$log"; then
        echo "FAIL $name: panicked during startup"
        grep -A2 "panicked" "$log" | head -4 | sed 's/^/    /'
        rc=1
    elif ! kill -0 "$pid" 2>/dev/null; then
        echo "FAIL $name: exited before the settle was over (see $log)"
        tail -3 "$log" | sed 's/^/    /'
        rc=1
    else
        echo "ok   $name"
    fi
    kill "$pid" 2>/dev/null
    wait "$pid" 2>/dev/null
done

for entry in "${SKIPPED[@]}"; do
    echo "skip ${entry%%|*}: ${entry#*|}"
done

exit "$rc"
