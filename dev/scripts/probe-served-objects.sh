#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Ask a running daemon which objects it actually serves.
#
# The defect this exists for, found on 11 August by starting a daemon and asking
# the bus: `arlen-ai-engine-daemon` OWNED `org.arlen.AI1` and served NOTHING at
# `/org/arlen/AI1`. The launcher's Ask pane came back with `Unknown object`. Three
# separate conditions could suppress the registration - AI switched off, the pi
# sidecar paths not resolving, the explain skill not found - while the bus name
# was claimed unconditionally, so a caller met a name that existed and a path that
# did not.
#
# **Owning the name is not the pass. Serving the path is.** That inversion is the
# whole point, the same way `probe-dbus-gate.sh` treats a method that ANSWERS as
# its failure.
#
# Why this is a probe and not a scanner in `dev/scripts/check-*`: registration is
# a RUNTIME act, and in that daemon it sat two conditionals deep. A source reader
# would have to guess which branch runs; a live daemon simply answers. Source is
# still the right place to learn what callers DIAL - that part is a literal in the
# caller - so the pairs are passed in rather than invented here.
#
# Usage:
#   dev/scripts/probe-served-objects.sh <daemon-binary> <bus-name> <path> [path...]
#
# Example:
#   dev/scripts/probe-served-objects.sh \
#       target/debug/arlen-ai-engine-daemon org.arlen.AI1 /org/arlen/AI1
#
# A private bus, never the developer's own session: these daemons take their name
# as sole owner, so probing a live session would either fail or take the name from
# the real one.
#
# Not in CI: it needs a built binary and a few seconds per daemon. It is the step
# between "the interface compiles" and "the object was watched being served".
set -uo pipefail

BIN="${1:?usage: probe-served-objects.sh <daemon-binary> <bus-name> <path> [path...]}"
NAME="${2:?a bus name is required}"
shift 2
PATHS=("$@")
[ "${#PATHS[@]}" -gt 0 ] || { echo "at least one object path is required" >&2; exit 2; }

[ -x "$BIN" ] || { echo "not executable: $BIN (cargo build first)" >&2; exit 2; }

if [ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ]; then
  exec dbus-run-session -- "$0" "$BIN" "$NAME" "${PATHS[@]}"
fi

# A private runtime dir too: these daemons bind sockets under it, and a probe must
# not collide with a live session's.
RUNTIME_DIR="$(mktemp -d)"
export XDG_RUNTIME_DIR="$RUNTIME_DIR"
chmod 700 "$RUNTIME_DIR"
LOG="$RUNTIME_DIR/daemon.log"

"$BIN" >"$LOG" 2>&1 &
DAEMON_PID=$!
cleanup() {
  kill "$DAEMON_PID" 2>/dev/null
  wait "$DAEMON_PID" 2>/dev/null
  rm -rf "$RUNTIME_DIR"
}
trap cleanup EXIT

# Wait for the NAME, which is the weaker half and the one that appears first. The
# whole finding is that it can appear alone, so this is a precondition, not a pass.
owned=0
for _ in $(seq 1 50); do
  if busctl --user list --no-legend 2>/dev/null | grep -q "^${NAME} "; then
    owned=1
    break
  fi
  sleep 0.2
done

if [ "$owned" -ne 1 ]; then
  echo "FAIL  ${NAME} never appeared on the bus; the daemon did not start or could not take the name"
  echo "--- daemon log:"
  tail -20 "$LOG"
  exit 1
fi

echo "${NAME} is owned. Now asking what it serves:"
failed=0
for path in "${PATHS[@]}"; do
  # Introspection lists the interfaces at a path. An unserved path answers
  # "Unknown object" on stderr and a non-zero status.
  if out="$(busctl --user introspect "$NAME" "$path" 2>&1)"; then
    methods="$(printf '%s\n' "$out" | grep -c ' method ')"
    # A path that introspects but carries no method is the standard properties
    # boilerplate and nothing of ours - served in name only.
    if [ "$methods" -eq 0 ]; then
      echo "  FAIL ${path} introspects but exposes no method; only the D-Bus boilerplate is there"
      failed=1
    else
      echo "  ok   ${path} (${methods} method(s))"
    fi
  else
    echo "  FAIL ${path} is dialled by a caller and NOT served: ${out}"
    failed=1
  fi
done

if [ "$failed" -ne 0 ]; then
  echo
  echo "A name without an object is an absence: the caller learns only that the path"
  echo "is missing, which is the least useful true thing we could tell it. Register the"
  echo "object whenever the connection exists and refuse with the real cause instead."
  echo "--- daemon log:"
  tail -20 "$LOG"
  exit 1
fi

echo "every dialled path is served"
