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
# A method that takes arguments carries them in its entry, as busctl wants them:
#
#   dev/scripts/probe-dbus-gate.sh target/debug/arlen-connectionsd \
#       org.arlen.Connections1 /org/arlen/Connections1 org.arlen.Connections1 \
#       "FetchEgressCredential sss anthropic api.anthropic.com bearer"
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

# `--all <file>`: every gated method in a tab-separated list, grouped by daemon so
# each one starts once. The list records what each method is EXPECTED to do, which
# is what lets this compare against intent instead of against a single blanket
# rule - two of the agent's reads answer on purpose, and a probe that cannot know
# that reports correct code as broken until somebody stops reading it.
if [ "${1:-}" = "--all" ]; then
  LIST="${2:?usage: probe-dbus-gate.sh --all <file>}"
  [ -r "$LIST" ] || { echo "cannot read $LIST" >&2; exit 2; }
  rc=0
  prev=""
  methods=()
  flush() {
    [ -n "$prev" ] || return 0
    IFS='|' read -r bin bus obj iface <<< "$prev"
    if [ ! -x "target/debug/$bin" ]; then
      echo "### $bin  SKIP (not built)"
      return 0
    fi
    echo "### $bin  $bus"
    "$0" "target/debug/$bin" "$bus" "$obj" "$iface" "${methods[@]}" || rc=1
  }
  while IFS=$'\t' read -r bin bus obj iface expect method; do
    case "${bin# }" in ''|\#*) continue ;; esac
    [ -n "${method:-}" ] || continue
    key="$bin|$bus|$obj|$iface"
    if [ "$key" != "$prev" ]; then
      flush
      prev="$key"
      methods=()
    fi
    case "$expect" in
      open:*) methods+=("[open] $method") ;;
      *)      methods+=("$method") ;;
    esac
  done < "$LIST"
  flush
  exit "$rc"
fi

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

# Sweep the five temp paths on the way out, whichever way that is. The script
# already kills the daemon at each of its three exits, so that part is covered;
# what none of them do is remove the four private XDG dirs and the log, and a
# cleanup line at the bottom would only reach the success path anyway. Four
# directories per run, on a probe meant to be run repeatedly while chasing a gate.
#
# The kill here is belt-and-braces for an exit those three do not own (an
# interrupt, or a future early return); it is harmless when the daemon is already
# gone, and `$DAEMON` being unset at that point is fine since this script sets
# neither `-e` nor `-u`.
#
# The daemon log survives a FAILURE and is named, because a probe reporting that
# a gate did not refuse is a probe whose log you want to read; on success there is
# nothing in it worth keeping.
cleanup() {
  status=$?
  kill "$DAEMON" 2>/dev/null
  rm -rf "$XDG_RUNTIME_DIR" "$XDG_DATA_HOME" "$XDG_CONFIG_HOME" "$XDG_STATE_HOME"
  if [ "$status" -eq 0 ]; then
    rm -f "$LOG"
  else
    echo "(kept for the failure: daemon log $LOG)" >&2
  fi
}
trap cleanup EXIT

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
untested=()
for m in "${METHODS[@]}"; do
  # A method entry may carry its arguments, exactly as busctl wants them:
  #   "FetchEgressCredential sss provider host scheme"
  # Without that the call goes out with no arguments, which only reaches a
  # method that takes none. The methods worth probing most - the ones handing
  # back a credential or a token - all take some, so a probe that could only
  # call the argument-free ones was covering the least sensitive half of every
  # interface it looked at.
  read -r -a spec <<< "$m"
  expect_open=0
  if [ "${spec[0]}" = "[open]" ]; then
    expect_open=1
    spec=("${spec[@]:1}")
  fi
  name="${spec[0]}"
  # Classify by OUTCOME, with "I could not tell" as the loud default.
  #
  # This matched failure MESSAGES until 12 Aug - `Signature mismatch`, then
  # `Invalid argument`, then two more when `Enact` answered `Too few parameters`.
  # Every wording it did not know fell through to ANSWERED, which is the quiet
  # verdict and the dangerous one: a gate reported as absent when it was never
  # consulted. Enumerating messages meant the list was always one wording behind
  # the next daemon. So the question is now asked the other way round - what do
  # we actually know? - and anything unrecognised fails the run instead of
  # passing it.
  # Unpiped, so `$?` is busctl's own exit and not `head`'s. Piping here made
  # every call look successful, which handed the classifier below the one input
  # that turns it back into the bug it was written to remove.
  out=$(timeout 10 busctl --user call "$BUS" "$OBJ" "$IFACE" "$name" "${spec[@]:1}" 2>&1)
  status=$?
  out=$(printf '%s' "$out" | head -1)
  # Either order, and both parts required: `refused Compensate` and `compensate
  # refused: ...` are both in the tree, written by daemons that never compared
  # notes, and an unrelated refusal logged nearby must not count.
  logged_refusal=0
  if grep -iE "\b$name\b" "$LOG" 2>/dev/null | grep -qiE "refus(ed|ing|al)"; then
    logged_refusal=1
  fi

  verdict="unknown"
  if [ "$logged_refusal" = "1" ]; then
    # The daemon says it refused - true whether it answered with a D-Bus error
    # or, like installd, returned `false` plus a reason.
    verdict="refused"
  elif [ "$status" -eq 0 ]; then
    verdict="answered"
  else
    case "$out" in
      *"Access denied"* | *"AccessDenied"*) verdict="refused" ;;
    esac
  fi

  case "$verdict" in
    refused)
      if [ "$expect_open" = "1" ]; then
        echo "  $name: REFUSED, but its entry says it should answer"
        answered+=("$name")
      else
        echo "  $name: refused"
      fi ;;
    answered)
      if [ "$expect_open" = "1" ]; then
        echo "  $name: answered, as its entry says it should"
      else
        echo "  $name: ANSWERED -> $out"
        answered+=("$name")
      fi ;;
    *)
      # The call failed and nothing said the gate was reached: a wrong signature,
      # a timeout, a daemon that died. Not a verdict either way.
      echo "  $name: NOT TESTED (the call failed and no refusal was logged) -> $out"
      untested+=("$name") ;;
  esac
done

if [ "${#untested[@]}" -gt 0 ]; then
  echo
  echo "not tested, because they take arguments this probe does not know: ${untested[*]}"
  echo "Pass a method that takes none, or extend the probe to send a valid signature."
fi

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
# An untested method is not a pass. Saying "every probed method refused" while
# three of the four were never actually called is the false-green this probe
# exists to prevent, aimed at itself.
if [ ${#untested[@]} -gt 0 ]; then
  echo
  echo "nothing was proven about: ${untested[*]}"
  exit 1
fi
echo "every probed method refused, and said so in the journal"
exit 0
