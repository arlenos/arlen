#!/usr/bin/env bash
# kill-dev.sh - free the Arlen dev stack when a "Port already in use" blocks a restart.
# Kills the dev servers (vite + esbuild + tauri) and the process-compose orchestrator
# spawned from this repo. Leaves long-running standalone daemons alone unless --all.
#
# Usage:
#   dev/kill-dev.sh                # all app dev servers + process-compose
#   dev/kill-dev.sh terminal files # only the named app(s)
#   dev/kill-dev.sh --all          # also kill arlen daemons running from this repo
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

all=0
apps=()
for arg in "$@"; do
  case "$arg" in
    --all) all=1 ;;
    *) apps+=("$arg") ;;
  esac
done

# App dev servers (vite/esbuild/node) hold the dev ports.
if [ "${#apps[@]}" -gt 0 ]; then
  alt=$(printf '%s|' "${apps[@]}"); alt="${alt%|}"
  patterns=("$REPO/apps/($alt)/")
else
  patterns=("$REPO/apps/" "process-compose")
fi
[ "$all" -eq 1 ] && patterns+=("$REPO/daemons/" "$REPO/ai/" "$REPO/target/")

pids=""
for p in "${patterns[@]}"; do
  pids="$pids $(pgrep -f "$p" 2>/dev/null || true)"
done
pids=$(echo "$pids" | tr ' ' '\n' | grep -v '^$' | sort -u)

if [ -z "$pids" ]; then
  echo "Nothing to kill (no Arlen dev processes for: ${apps[*]:-all})."
  exit 0
fi

echo "Killing Arlen dev processes:"
for pid in $pids; do
  echo "  $pid  $(ps -o args= -p "$pid" 2>/dev/null | cut -c1-90)"
done

kill $pids 2>/dev/null
sleep 1
for pid in $pids; do kill -0 "$pid" 2>/dev/null && kill -9 "$pid" 2>/dev/null; done

echo "Done - dev ports freed."
