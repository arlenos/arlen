#!/usr/bin/env bash
# Sync the canonical UI primitives from `sdk/ui-kit` into the
# consuming apps (`app-settings`, `desktop-shell`).
#
# We deliberately keep three filesystem copies — Tauri's bundler +
# Tailwind v4's class-hash scoping break across symlinked
# components, so each app needs its own physical copy. Until we
# convert the workspace to a real npm package dependency, this
# script is the drift firewall.
#
# Usage:
#   ./distro/sync-ui-kit.sh           # copy canonical → consumers
#   ./distro/sync-ui-kit.sh --check   # diff only, exit 1 on drift
#
# The list of tracked primitives is the intersection of the three
# `src/lib/components/ui/` directories. New primitives in sdk/ui-kit
# auto-propagate; primitives that exist only in a consumer (e.g.
# the settings-only `color-picker`) are never touched.

set -e

ROOT="$HOME/Repositories/lunaris-sys"
SRC="$ROOT/sdk/ui-kit/src/lib/components/ui"
CONSUMERS=(
  "$ROOT/app-settings/src/lib/components/ui"
  "$ROOT/desktop-shell/src/lib/components/ui"
)

CHECK_MODE=0
if [ "${1:-}" = "--check" ]; then
  CHECK_MODE=1
fi

if [ ! -d "$SRC" ]; then
  echo "error: canonical ui-kit not found at $SRC" >&2
  exit 1
fi

DRIFT=0

for consumer in "${CONSUMERS[@]}"; do
  if [ ! -d "$consumer" ]; then
    echo "skip: $consumer (not present)"
    continue
  fi
  for src_sub in "$SRC"/*/; do
    name=$(basename "$src_sub")
    dst_sub="$consumer/$name"
    if [ ! -d "$dst_sub" ]; then
      # Consumer doesn't have this primitive — skip rather than
      # populate, in case the consumer omits primitives on purpose.
      continue
    fi
    if [ $CHECK_MODE -eq 1 ]; then
      if ! diff -rq "$src_sub" "$dst_sub" > /dev/null 2>&1; then
        echo "DRIFT: $name in $consumer"
        diff -rq "$src_sub" "$dst_sub" | head -5
        DRIFT=1
      fi
    else
      rsync -a --delete "$src_sub" "$dst_sub"
      echo "synced: $name → $consumer"
    fi
  done
done

if [ $CHECK_MODE -eq 1 ] && [ $DRIFT -eq 1 ]; then
  exit 1
fi
echo "done."
