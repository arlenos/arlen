#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# The sensing switch reader exists three times: Settings, the xdg portal and the
# compositor. They are held to one table of vectors rather than merged, because a
# cross-repo release dependency for four lines costs more than it buys.
#
# Each repository's tests answer its own copy of that table, so a copy that drifts
# still passes on both sides while the two now describe different rules. This is
# the only check that sees both, so it is the only one that catches that.
#
# Skips when the compositor is not checked out. That is honest rather than
# convenient: this cannot run in a single-repo CI, and pretending otherwise would
# make a green run mean less than it does.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ARLEN_VECTORS="$HERE/../fixtures/sensing-vectors"
COMPOSITOR_PATH="${COMPOSITOR_PATH:-$HOME/Repositories/compositor}"
COMPOSITOR_VECTORS="$COMPOSITOR_PATH/dev/fixtures/sensing-vectors"

[ -d "$ARLEN_VECTORS" ] || { echo "no vector table at $ARLEN_VECTORS" >&2; exit 1; }

if [ ! -d "$COMPOSITOR_VECTORS" ]; then
  echo "compositor not checked out at $COMPOSITOR_PATH; sensing vectors unchecked" >&2
  exit 0
fi

# Compare the switch files only. The README is prose and lives in one repo.
if diff -r --exclude=README.md "$ARLEN_VECTORS" "$COMPOSITOR_VECTORS"; then
  n=$(find "$ARLEN_VECTORS" -name '*.toml' | wc -l)
  echo "sensing vectors agree across both repositories ($n cases)"
else
  cat >&2 <<'MSG'

The two copies of the sensing vector table have diverged. Whichever is right, the
three readers are no longer held to one rule, and a master switch that enforces on
two paths out of three is not a master switch.

Copy the intended table over the other and run both test suites:
  cargo test --manifest-path daemons/xdg-portal/daemon/Cargo.toml sensing
  cargo test --manifest-path apps/settings/src-tauri/Cargo.toml sensing
  ( cd "$COMPOSITOR_PATH" && cargo test --lib sensing )
MSG
  exit 1
fi
