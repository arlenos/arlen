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
# What this does NOT do is read the logs. Tried it: every daemon here starts
# ALONE in its own throwaway runtime dir, so the ones with peers log exactly what
# you would expect - anomalyd cannot reach the audit socket, code-indexer cannot
# reach the event bus, the graph writer logs ERROR reconnecting to a bus nobody
# started. Surfacing that is three lines of noise on a healthy run, and noise on
# a healthy run is how a person learns to stop reading the output. The health
# signal available to a single-daemon smoke is the one it already asserts: it
# came up, it bound what it claims, and it was still alive after.
#
# Daemons NOT started here each carry their reason in SKIPPED below, and
# `check-smoke-coverage.py` fails if a daemon binary is in neither list. This
# comment used to carry the reasons in prose and named eight of the twenty-four
# it excluded, which is the failure it warned about: a smoke test that skips
# things silently is worse than one that does not exist, and one that CLAIMS to
# list its exclusions while listing a third of them is worse again.
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
    "arlen-modulesd|modulesd.sock|"
    # The knowledge daemon needs its store paths pointed somewhere disposable,
    # and the FUSE timeline turned OFF: left at its default it would try to mount
    # over the dev's ~/.timeline. It logs consumer-reconnect errors while the
    # event bus is absent, which is the writer's backoff working, not a failure -
    # the log is only printed when the socket never appears.
    "arlen-anomalyd|-|"
    "arlen-code-indexer|-|"
    "arlen-journald-parser|-|"
    "arlen-transferd|-|"
    "arlen-knowledge-mcp|-|"
    "arlen-file-manager-mcp|-|"
    "arlen-system-monitor-mcp|-|"
    "arlen-terminal-run-mcp|-|"
    # Takes its config from the environment, so the bundled Obsidian example
    # drives it: it comes up as a real bridge and watches an empty vault.
    "arlen-bridge-ingest|-|ARLEN_BRIDGE_CONFIG=daemons/bridge-ingest/examples/obsidian/bridge.toml ARLEN_OBSIDIAN_VAULT=\$rt/vault"
    "arlen-graph-daemon|knowledge.sock|ARLEN_DB_PATH=\$rt/knowledge/events.db ARLEN_GRAPH_PATH=\$rt/knowledge/graph ARLEN_TIMELINE_MOUNT=off ARLEN_PERMISSIONS_DIR=\$rt/permissions"
)

# name|reason - every daemon binary that is deliberately not started here. The
# coverage check reads this, so adding a daemon means classifying it.
SKIPPED=(
    "arlen-powerd|needs the system bus (UPower, logind)"
    "arlen-accountsd|needs the session bus"
    "arlen-notifyd|needs the session bus (org.freedesktop.Notifications)"
    "arlen-clockd|needs the session bus (it owns org.arlen.Clock1)"
    "xdg-desktop-portal-arlen|needs the session bus and the portal frontend"
    "arlen-installd|needs the session bus"
    "arlen-install-helper|system-bus service, runs as root"
    "arlen-permission-helper|system-bus service, runs as root"
    "arlen-connectionsd|needs the session bus"
    "arlen-config-broker|runs as its own uid by design"
    "kernel-layer|needs the bpf toolchain and a privileged host"
    "kernel-layer-ebpf|an eBPF object, not a host binary"
    "arlen-run|a launcher: it execs a confined app and exits"
    "arlen-ai-engine-daemon|needs the session bus and a provisioned model"
    "arlen-wallpaperd|exits cleanly with no manifest configured, so there is nothing to stay up for"
    "arlen-timeline|mounts a FUSE filesystem, which needs /dev/fuse and a mount point the run would have to own"
)

failed=0
started=0
unbuilt=()
for entry in "${DAEMONS[@]}"; do
    IFS='|' read -r name sock extra <<<"$entry"
    bin="target/debug/$name"
    # An unbuilt binary is NOT a legitimate skip. The legitimate ones are in
    # SKIPPED with their reason; DAEMONS is the list this run has to exercise, so
    # a missing binary means the run did not do its job. It used to print SKIP and
    # carry on, which on a tree with nothing built reported `OK: 0 daemon(s)
    # started` and exited 0 - a gate passing while testing nothing, which is the
    # failure this file's own header calls worse than having no smoke at all.
    if [ ! -x "$bin" ]; then
        echo "MISS $name: not built"
        unbuilt+=("$name")
        continue
    fi
    rt=$(mktemp -d "${TMPDIR:-/tmp}/arlen-smoke.XXXXXX")
    chmod 700 "$rt"
    mkdir -p "$rt/arlen" "$rt/knowledge" "$rt/vault"
    # `extra` carries `$rt` unexpanded so it can be resolved against THIS run's
    # directory; eval is safe here because the list is in this file, not input.
    eval "extra_env=\"$extra\""
    # shellcheck disable=SC2086
    env XDG_RUNTIME_DIR="$rt" XDG_DATA_HOME="$rt/data" XDG_STATE_HOME="$rt/state" \
        XDG_CONFIG_HOME="$rt/config" $extra_env "$bin" >"$rt/log" 2>&1 </dev/null &
    pid=$!
    # A socket of `-` means this daemon serves nothing: it consumes events, polls
    # a ledger or waits on a bus. There is no bind to wait for, so the assertion
    # is that it is still running after a settle rather than having panicked its
    # way out, which is the failure this whole script exists to catch.
    if [ "$sock" = "-" ]; then
        sleep 2
        if kill -0 "$pid" 2>/dev/null; then
            echo "OK   $name -> running (serves no socket)"
            started=$((started + 1))
        else
            echo "FAIL $name: exited during startup"
            sed 's/^/       /' "$rt/log" | head -5
            failed=1
        fi
    else
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
    fi
    kill "$pid" 2>/dev/null
    wait "$pid" 2>/dev/null
    rm -rf "$rt"
done

if [ "${#unbuilt[@]}" -ne 0 ]; then
    echo
    echo "${#unbuilt[@]} of ${#DAEMONS[@]} daemon(s) are not built, so this run proved nothing about them:"
    for name in "${unbuilt[@]}"; do
        # Resolved on the failure path only, so the list needs no fourth field to
        # drift: the manifest that declares the binary name is the crate to build.
        manifest=$(git grep -lF "\"$name\"" -- '*/Cargo.toml' | head -1)
        echo "  cargo build --manifest-path ${manifest:-<crate>/Cargo.toml} --bin $name"
    done
    exit 1
fi

if [ "$failed" -ne 0 ]; then
    echo "a daemon did not come up"
    exit 1
fi
echo "OK: $started of ${#DAEMONS[@]} daemon(s) started; each bound its socket or stayed up"
