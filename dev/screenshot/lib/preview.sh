# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Serving an app's built frontend for a drive, and stopping it again.
#
# WHY IT IS SHARED, and it is not tidiness. Two drives had this inline and
# identical, and both leaked: they wrote
#
#     (cd "$app" && npx vite preview --port 1421 ...) &
#     preview_pid=$!
#
# and killed `$preview_pid` at exit. That pid is the SUBSHELL's. `npx` spawns
# node inside it, node is what holds the port, and killing the subshell leaves
# node listening. Measured on 5 September: a preview from an earlier run was
# still on 1421 hours later.
#
# THE LEAK IS NOT THE WORST OF IT. `wait_for_http` cannot tell whose server
# answered, so the NEXT run of either drive sails past its readiness check
# against the stale one - and reads a frontend built at some earlier time. Every
# assertion below that point is then about a page nobody just built, which is the
# stale-binary failure one layer out: a suite that passes while testing something
# other than what is in the tree.
#
# So this refuses to run against a server it did not start, rather than trusting
# a port that answers.
#
# A THIRD COPY EXISTS, and saying so is better than quietly leaving it.
# `shoot-no-backend.sh` worked this out first - its comment describes the exact
# failure ("the screenshot is of that server's build, the previous one, taken
# after the fix and looking exactly like a verification of it") and its stop
# VERIFIES the port went quiet, which this one did not until it was read. That
# check is now here too.
#
# It is not folded in, because the two differ on a POLICY rather than a
# mechanism: it CLEARS a leftover port and then refuses if the clearing failed,
# where this refuses outright. Clearing is kinder after an interrupted run;
# refusing never signals a process this run did not start, which is the rule that
# governs the fixture helper and the one this file learnt the hard way when an
# earlier cut killed its own caller. Picking one is a decision about whose port
# 1421 is, and it belongs to whoever owns that call - not to a tidy-up.

# Serve $1's `build/` on port $2. Sets PREVIEW_PGID in the CALLER's shell.
#
# Not echoed and not captured in `$( )`: that subshell is where the last pid went
# to die (see lib/bus.sh, same day, same mistake).
start_preview() {
    _pv_app="$1"
    # Kept for stop_preview, which verifies THIS port went quiet.
    _pv_port="$2"
    if curl -sS -o /dev/null --max-time 2 "http://localhost:$_pv_port/" 2>/dev/null; then
        echo "!! something is already serving port $_pv_port, and this drive did not" >&2
        echo "   start it. Refusing rather than testing whatever it is: a leftover" >&2
        echo "   preview serves a frontend from whenever it was built, and every" >&2
        echo "   assertion after this would be about that page instead of yours." >&2
        echo "   ps -eo pid,args | grep '[v]ite preview'  - then kill it." >&2
        return 1
    fi
    # `setsid` makes the server a process-group leader, so ONE kill takes npx and
    # the node it spawns. Without it the group is this script's and killing it
    # would take the drive down with the server.
    ( cd "$_pv_app" && exec setsid npx vite preview --port "$_pv_port" --strictPort \
        --outDir build >/dev/null 2>&1 ) &
    _pv_wrapper=$!
    # The group id is the setsid child's, which is NOT $! - that is the subshell
    # wrapping it. So it is found by what it is listening for.
    #
    # AND IT MUST NOT BE OURS. The first cut matched the wrapper before `setsid`
    # had taken effect, so PREVIEW_PGID came back as this script's own group -
    # and `stop_preview` then killed the caller. It killed me, once, which is how
    # I know. A helper that can end its caller is the fixture-delete rule wearing
    # a different hat: it may only ever act on something it made. Skipping our own
    # group also makes the wait correct rather than lucky - it now waits for the
    # process that is genuinely detached, instead of taking the first match.
    _pv_self="$(ps -o pgid= -p $$ | tr -d ' ')"
    for _ in $(seq 1 60); do
        PREVIEW_PGID="$(ps -eo pgid,args --no-headers \
            | awk -v p="--port $_pv_port" -v self="$_pv_self" \
                  '$0 ~ p && $0 !~ /awk/ && $1 != self {print $1; exit}')"
        [ -n "${PREVIEW_PGID:-}" ] && break
        sleep 0.25
    done
    unset _pv_wrapper
    if [ -z "${PREVIEW_PGID:-}" ] || [ "$PREVIEW_PGID" = "$_pv_self" ]; then
        echo "!! the preview never detached into its own process group; refusing" >&2
        echo "   to record a group id, because stopping it would end this script." >&2
        PREVIEW_PGID=""
        return 1
    fi
    return 0
}

# Stop the preview started by [start_preview], group and all.
stop_preview() {
    [ -n "${PREVIEW_PGID:-}" ] || return 0
    # Belt and braces on the same rule: never signal our own group, whatever is
    # in the variable by the time we get here.
    if [ "$PREVIEW_PGID" = "$(ps -o pgid= -p $$ | tr -d ' ')" ]; then
        echo "!! refusing to stop the preview: that group is this script's" >&2
        PREVIEW_PGID=""
        return 1
    fi
    _pv_group="$PREVIEW_PGID"
    PREVIEW_PGID=""
    kill -- "-$_pv_group" 2>/dev/null
    # VERIFY, do not trust the kill. `shoot-no-backend.sh` had worked this out
    # first and this helper had not: it escalates and then says so if the port is
    # still answering. A stop that returns without checking is how the port stays
    # occupied and the NEXT run reads someone else's frontend, which is the whole
    # defect this file exists for - so the check belongs here rather than in the
    # one script that happened to think of it.
    for _ in $(seq 1 20); do
        curl -sf -o /dev/null "http://localhost:$_pv_port/" 2>/dev/null || return 0
        sleep 0.25
    done
    kill -9 -- "-$_pv_group" 2>/dev/null
    for _ in $(seq 1 20); do
        curl -sf -o /dev/null "http://localhost:$_pv_port/" 2>/dev/null || return 0
        sleep 0.25
    done
    echo "!! a preview still answers on $_pv_port after SIGKILL. The next run will" >&2
    echo "   refuse rather than test it, which is the right failure, but this one" >&2
    echo "   left something behind: ps -eo pid,args | grep '[v]ite preview'" >&2
    return 1
}
