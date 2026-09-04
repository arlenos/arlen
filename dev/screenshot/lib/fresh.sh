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
