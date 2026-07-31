#!/bin/bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Start each daemon that can run unattended, and check it binds what it claims.
#
# Unit tests never start a process, so a daemon can compile, pass its suite and
# still die on startup - a missing directory, a socket path it cannot create, a
# config read that panics. Nothing else in the gate covers that, and `just test`
# by construction cannot: it tests functions, not processes.
#
# Each daemon runs with a throwaway XDG_RUNTIME_DIR so it never touches the
# dev's real sockets, and is killed after it has either bound its socket or run
# out of time. Nothing is installed, nothing persists.
#
# Daemons NOT started here, with the reason, because a smoke test that skips
# things silently is worse than one that does not exist:
#   - anything needing a session or system bus (power, online-accounts,
#     notification, xdg-portal, installd): a bus is not available unattended
#   - config-broker: runs as its own uid by design
#   - kernel-layer: needs the bpf toolchain
#   - bridge-ingest: needs a bridge.toml argument; covered by its own example
#     in the crate, not by a socket bind
set -uo pipefail
cd "$(dirname "$0")/../.." || exit 1

# name|socket-basename|extra env - the socket the daemon says it binds, and any
# env it needs to keep its state inside the throwaway dir. `$rt` expands per run.
DAEMONS=(
    "event-bus|event-bus-producer.sock|"
    "arlen-consent-broker|consent-intake.sock|"
    "arlen-store-backend|store.sock|"
    "arlen-auditd|audit-ingest.sock|"
    "arlen-capsuled|capsule.sock|"
    "arlen-settings-broker|settings-broker.sock|"
    # The knowledge daemon needs its store paths pointed somewhere disposable,
    # and the FUSE timeline turned OFF: left at its default it would try to mount
    # over the dev's ~/.timeline. It logs consumer-reconnect errors while the
    # event bus is absent, which is the writer's backoff working, not a failure -
    # the log is only printed when the socket never appears.
    "arlen-graph-daemon|knowledge.sock|ARLEN_DB_PATH=\$rt/knowledge/events.db ARLEN_GRAPH_PATH=\$rt/knowledge/graph ARLEN_TIMELINE_MOUNT=off ARLEN_PERMISSIONS_DIR=\$rt/permissions"
)

failed=0
started=0
for entry in "${DAEMONS[@]}"; do
    IFS='|' read -r name sock extra <<<"$entry"
    bin="target/debug/$name"
    if [ ! -x "$bin" ]; then
        echo "SKIP $name: not built (cargo build --manifest-path <its crate>)"
        continue
    fi
    rt=$(mktemp -d "${TMPDIR:-/tmp}/arlen-smoke.XXXXXX")
    chmod 700 "$rt"
    mkdir -p "$rt/arlen" "$rt/knowledge"
    # `extra` carries `$rt` unexpanded so it can be resolved against THIS run's
    # directory; eval is safe here because the list is in this file, not input.
    eval "extra_env=\"$extra\""
    # shellcheck disable=SC2086
    env XDG_RUNTIME_DIR="$rt" XDG_DATA_HOME="$rt/data" XDG_STATE_HOME="$rt/state" \
        $extra_env "$bin" >"$rt/log" 2>&1 &
    pid=$!
    # Poll rather than sleep a fixed time: the bind is fast, and a fixed wait
    # either flakes or wastes seconds. Ten half-seconds is generous.
    bound=0
    for _ in $(seq 20); do
        [ -S "$rt/arlen/$sock" ] && { bound=1; break; }
        kill -0 "$pid" 2>/dev/null || break
        sleep 0.5
    done
    if [ "$bound" -eq 1 ]; then
        echo "OK   $name -> $sock"
        started=$((started + 1))
    else
        echo "FAIL $name: never bound $sock"
        sed 's/^/       /' "$rt/log" | head -5
        failed=1
    fi
    kill "$pid" 2>/dev/null
    wait "$pid" 2>/dev/null
    rm -rf "$rt"
done

if [ "$failed" -ne 0 ]; then
    echo "a daemon did not come up"
    exit 1
fi
echo "OK: $started daemon(s) started and bound their socket"
