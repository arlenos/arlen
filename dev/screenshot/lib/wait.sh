# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Waiting for a thing rather than for a number, for the drive scripts.
#
# WHY IT IS SHARED. Two drives started a preview server and slept two seconds,
# and on 4 September that cost a cycle: the run right after a fresh frontend
# build came up slower than the sleep, the app loaded nothing, and the drive
# reported a failure in the PAGE about the DAEMON - a false red naming the wrong
# component, which is worse than a slow script. Fixing it in one place and
# copying it to the other would have been two chances to fix only one.
#
# `shoot-no-backend.sh` already polls with curl; this is that pattern, named, so
# a third drive gets it by sourcing rather than by remembering.

# Wait until an HTTP server answers on $1, or fail after ~30s.
wait_for_http() {
    url="$1"
    for _ in $(seq 1 60); do
        curl -sS -o /dev/null "$url" 2>/dev/null && return 0
        sleep 0.5
    done
    echo "!! nothing answered at $url; nothing below is about the app" >&2
    return 1
}

# Wait until a Unix socket exists at $1, or fail after ~10s.
wait_for_socket() {
    path="$1"
    for _ in $(seq 1 40); do
        [ -S "$path" ] && return 0
        sleep 0.25
    done
    echo "!! no socket at $path; nothing below is about what it serves" >&2
    return 1
}
