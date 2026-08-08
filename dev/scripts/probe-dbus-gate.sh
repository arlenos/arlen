#!/bin/bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Start a daemon on a PRIVATE session bus and check that its gated methods refuse
# a caller they should not serve.
#
# Usage:
#   dbus-run-session -- dev/scripts/probe-dbus-gate.sh \
#       target/debug/arlen-undod org.arlen.Undo1 /org/arlen/Undo1 org.arlen.Undo1 \
#       Recent Enact
#
# `busctl` is the caller, and it is not a first-party surface: its binary does not
# resolve to an app id, so every gated method must refuse it. **A method that
# ANSWERS is the failure.** That inversion is the whole point - a probe where the
# call succeeding counted as a pass would be measuring that the daemon is up, not
# that its gate does anything.
#
# A private bus, never the developer's own session: these daemons take their name
# as sole owner and refuse to start when it is held, so probing a live session
# would either fail or take the name from the real one.
#
# Not in CI: it needs a built binary and about five seconds per daemon. It is the
# step between "the gate compiles" and "the gate was watched refusing something",
# which this project has needed more than once today.
#
# Shown to fail before being trusted, on the two daemons it was written for:
#
#   arlen-undod     Recent                     -> refused, exit 0
#   arlen-installd  ListInstalled ListTrashed  -> refused, exit 0
#   arlen-undod     org.freedesktop.DBus.Peer Ping -> ANSWERED, exit 1
#
# The last one is the control: Ping is not ours and not gated, so a probe that
# could not tell it apart from a gated method would be reporting nothing.

set -u

BIN="${1:?usage: probe-dbus-gate.sh <binary> <bus-name> <object> <interface> <method>...}"
BUS="${2:?bus name}"
OBJ="${3:?object path}"
IFACE="${4:?interface}"
shift 4
METHODS=("$@")
[ ${#METHODS[@]} -gt 0 ] || { echo "name at least one method to probe" >&2; exit 2; }

[ -x "$BIN" ] || { echo "no such binary: $BIN (build it first)" >&2; exit 2; }
[ -n "${DBUS_SESSION_BUS_ADDRESS:-}" ] || {
  echo "run this under dbus-run-session: a private bus, not yours" >&2
  exit 2
}

# Private state directories too. A probe that writes into the developer's real
# XDG dirs is a probe that changes the machine it is measuring.
export XDG_RUNTIME_DIR=$(mktemp -d)
export XDG_DATA_HOME=$(mktemp -d)
export XDG_CONFIG_HOME=$(mktemp -d)
export XDG_STATE_HOME=$(mktemp -d)
export RUST_LOG="${RUST_LOG:-info}"

LOG=$(mktemp)
"$BIN" >"$LOG" 2>&1 &
DAEMON=$!
sleep 3

if ! busctl --user list 2>/dev/null | grep -q "$BUS"; then
  echo "$BUS never appeared on the bus - the daemon did not start:"
  sed 's/^/  /' "$LOG" | tail -10
  kill "$DAEMON" 2>/dev/null
  exit 2
fi
echo "$BUS is owned by $(basename "$BIN")"

answered=()
for m in "${METHODS[@]}"; do
  out=$(timeout 10 busctl --user call "$BUS" "$OBJ" "$IFACE" "$m" 2>&1 | head -1)
  case "$out" in
    *"Access denied"* | *"AccessDenied"*)
      echo "  $m: refused" ;;
    *)
      echo "  $m: ANSWERED -> $out"
      answered+=("$m") ;;
  esac
done

echo "refusals in the journal:"
grep -i "refus" "$LOG" | sed 's/^/  /' | tail -10 || echo "  none logged"

kill "$DAEMON" 2>/dev/null
wait 2>/dev/null

if [ ${#answered[@]} -gt 0 ]; then
  echo
  echo "these served a caller that does not resolve to an app id: ${answered[*]}"
  echo "Either the gate is not reached on that path, or the method is not gated."
  exit 1
fi
echo "every probed method refused, and said so in the journal"
exit 0
