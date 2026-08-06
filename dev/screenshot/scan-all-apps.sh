#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Run the message-id scan over every app's routes, not just the two I happened
# to be working on.
#
# The reason this exists: on 6 August the Files sidebar was found showing
# `f.places.places` as a heading, because the store pushed a message id into a
# kit component that renders it verbatim. That is exactly what
# `scan-message-ids.sh` was written to catch, and it had never been pointed at
# the Files app - only at Settings and the shell. A check that covers two of
# eleven apps is not a check on the other nine, and nothing said so.
#
#   dev/screenshot/scan-all-apps.sh            # every app
#   dev/screenshot/scan-all-apps.sh files      # one app
#
# Each app's dev server is started, scanned and stopped in turn: several share a
# port (greeter, system-monitor and viewers are all on 1429), so they cannot run
# at once. Exits non-zero if any route showed an id.
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"

apps=("$@")
if [ "${#apps[@]}" -eq 0 ]; then
  mapfile -t apps < <(cd "$root/apps" && for d in */; do
    [ -f "${d}src-tauri/tauri.conf.json" ] && basename "$d"
  done)
fi

failed=0
scanned=0
for app in "${apps[@]}"; do
  dir="$root/apps/$app"
  conf="$dir/src-tauri/tauri.conf.json"
  [ -f "$conf" ] || { echo "skip  $app (no tauri.conf.json)"; continue; }

  port="$(grep -oE '"devUrl": "http://localhost:[0-9]+' "$conf" | grep -oE '[0-9]+$')"
  [ -n "$port" ] || { echo "skip  $app (no devUrl port)"; continue; }

  # Routes with a page, minus the `_`-prefixed dev surfaces: those are guarded
  # to 404 outside dev, and the lint already excludes them for the same reason.
  mapfile -t routes < <(cd "$dir/src/routes" 2>/dev/null && {
    [ -f "+page.svelte" ] && echo "/"
    for d in */; do
      n="${d%/}"
      case "$n" in _*) continue ;; esac
      [ -f "${d}+page.svelte" ] && echo "/$n"
      for s in "$d"*/; do
        [ -d "$s" ] || continue
        [ -f "${s}+page.svelte" ] && echo "/${s%/}"
      done
    done
  })
  [ "${#routes[@]}" -gt 0 ] || { echo "skip  $app (no routes)"; continue; }

  echo "── $app (port $port, ${#routes[@]} route(s))"
  ( cd "$dir" && npm run dev >"/tmp/scan-$app.log" 2>&1 ) &
  server=$!

  up=0
  for _ in $(seq 1 40); do
    sleep 1
    if [ "$(curl -s -o /dev/null -w '%{http_code}' "http://localhost:$port/" 2>/dev/null)" = "200" ]; then
      up=1
      break
    fi
  done

  if [ "$up" -eq 1 ]; then
    "$here/scan-message-ids.sh" "http://localhost:$port" "${routes[@]}"
    [ "$?" -ne 0 ] && failed=1
    scanned=$((scanned + 1))
  else
    # Say so rather than pass. A server that never came up scans as clean, which
    # is the false green this whole family of checks keeps running into.
    echo "FAIL  $app never served on $port; see /tmp/scan-$app.log"
    failed=1
  fi

  kill "$server" 2>/dev/null
  pkill -f "vite.*$port" 2>/dev/null
  wait "$server" 2>/dev/null
done

echo
echo "scanned $scanned app(s)"
exit "$failed"
