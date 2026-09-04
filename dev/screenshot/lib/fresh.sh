# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Is this binary newer than the code it was built from?
#
# WHY IT IS SEPARATE FROM shoot-app.sh's CHECK. That one covers the APP a drive
# photographs, and it earned its place: a stale one turns a suite into a test of
# last month's app, which cost most of a cycle on 4 September and now refuses
# outright. But several drives also SPAWN a daemon - sentineld, bottled, notifyd,
# the event bus, the graph - and every one of them checked only that the binary
# EXISTS. A daemon older than its source answers with last week's behaviour and
# the suite reports on that, with nothing anywhere saying so.
#
# The sentinel page is the sharpest case and the reason this exists: it is the
# surface that tells somebody their machine is protected, its whole point is that
# it must not say something nobody measured, and it was driven against a daemon
# whose age nothing checked.
#
# WHY MTIMES rather than a hash or a build stamp: a `git checkout`, a branch
# switch or a stash pop makes this fire without anybody editing anything, which
# is the cheap direction. A false alarm costs a rebuild; a missing one costs a
# green run about code that is not in the tree.

# Fail if $1 is older than any `.rs` under the directories that follow.
#
# Prints what changed, because "rebuild something" is not an instruction and the
# next question is always which file made it stale.
require_fresh() {
    _fr_bin="$1"
    shift
    if [ ! -x "$_fr_bin" ]; then
        echo "!! no binary at $_fr_bin" >&2
        return 1
    fi
    _fr_newer="$(find "$@" -name '*.rs' -newer "$_fr_bin" -print -quit 2>/dev/null || true)"
    if [ -n "$_fr_newer" ]; then
        echo "!! $(basename "$_fr_bin") is OLDER than its source ($_fr_newer changed" >&2
        echo "   since it was built). It would answer with the old behaviour and this" >&2
        echo "   suite would report on that. Rebuild it first." >&2
        return 1
    fi
    return 0
}

# Fail if the built frontend under $1 is older than any source under the rest.
#
# WHY THIS HALF WAS MISSING, and it cost a whole verification. `require_fresh`
# reads `.rs`, and a drive whose frontend is SERVED from a preview
# (`SHOOT_FRONTEND_SERVED=1`) deliberately compares the Rust only, because the
# binary genuinely does not carry the frontend on that path. What nothing then
# checked is the thing actually on screen: `vite preview` serves `build/`, and
# `build/` is written by a command nobody is forced to run. On 4 September the
# mail drive photographed a frontend eighty-nine minutes old, passed, and its
# verdict was about a page no longer in the tree.
#
# Same mtime reasoning as above: a checkout makes it fire, which is the cheap
# direction to be wrong in.
require_fresh_frontend() {
    _ff_build="$1"
    shift
    if [ ! -f "$_ff_build/index.html" ]; then
        echo "!! no built frontend at $_ff_build (run: npm run build)" >&2
        return 1
    fi
    _ff_newer="$(find "$@" \
        \( -name '*.svelte' -o -name '*.ts' -o -name '*.js' -o -name '*.css' -o -name '*.html' \) \
        -newer "$_ff_build/index.html" -print -quit 2>/dev/null || true)"
    if [ -n "$_ff_newer" ]; then
        echo "!! the built frontend at $_ff_build is OLDER than its source" >&2
        echo "   ($_ff_newer changed since). The preview would serve that build and" >&2
        echo "   this suite would report on a page that is not in the tree." >&2
        echo "   Rebuild it first: npm run build" >&2
        return 1
    fi
    return 0
}
