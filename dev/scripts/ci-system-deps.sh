#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Install the system libraries every CI job needs, taking the package bodies
# from a cache instead of a mirror.
#
# WHY THIS EXISTS. `system deps` failed a different job on every run for three
# runs running - contracts/file-change, then daemons/online-accounts, each time
# "timed out after 10 minutes", each time nothing to do with the code under
# test. A board that is red for a reason nobody needs to read is a board nobody
# reads, so the fix has to remove the cause rather than raise the ceiling again.
#
# The cause is arithmetic. One matrix produces 118 rust jobs, each fetching the
# same ~200 MB of packages from the same Azure mirror at the same moment;
# libwebkit2gtk-4.1-dev alone is 8 MB and it is the one that was measured
# stalling. Whichever handful of jobs the mirror happens to serve slowest are
# the ones that go red, which is exactly the pattern seen: three jobs one run,
# six the next, a different set each time.
#
# WHAT THE CACHE REMOVES: the package bodies. The 200 MB of `.deb` files was
# what sat on the critical path, and on a warm cache apt finds every one of them
# already in `/var/cache/apt/archives` and downloads nothing.
#
# That left the index, and the index turned out to be enough on its own to fail
# a job - see the warm-first install below, which is the answer to it. A cold
# run still fetches the index, and the retry around the whole call stays for
# that: a bounded attempt against a fresh mirror is what a slow index deserves.
#
# ONE PACKAGE SET FOR EVERY JOB, deliberately. The lint job needs a strict
# subset, and installing the extra handful there costs nothing from a cache
# while giving all three jobs the same cache key - so one entry serves the
# whole matrix rather than two entries serving parts of it. A second list would
# be a second thing to keep in step with this one, and the tree already has a
# check for that class of drift because it keeps happening.
#
# The first run after a change here is cold for every job in it, since a cache
# entry is only saved when a job finishes. That run leans on the retry, and the
# ones after it do not touch the mirror for a package body at all.
#
# Run: dev/scripts/ci-system-deps.sh
#
# not-a-local-gate: it installs packages with sudo, and a laptop has no apt.
set -uo pipefail

# cmake + nasm build AWS-LC, which aws-lc-rs (pulled in by the tough TUF
# library, used by forage/cookbook-sign) compiles from C source.
#
# libheif-dev and libheif-plugin-libde265 are the viewer's HEIC decoder;
# check-linked-libraries.py holds this list against the crates that link them.
# The list lives beside this script, so the workflow's cache key can hash what it
# is a cache OF. See `ci-system-packages.txt` for why.
PACKAGE_LIST="$(dirname "$0")/ci-system-packages.txt"
[ -f "$PACKAGE_LIST" ] || { echo "missing $PACKAGE_LIST" >&2; exit 1; }
PACKAGES="$(sed -e 's/#.*//' -e '/^[[:space:]]*$/d' "$PACKAGE_LIST" | tr '\n' ' ')"

# Where the workflow's cache step restores to and saves from. A directory the
# runner user owns, because `/var/cache/apt/archives` is root's and the cache
# action runs unprivileged.
CACHE="${APT_CACHE_DIR:-$HOME/apt-archive-cache}"

# apt's own download directory. Overridable only so the control script can
# exercise the copy in both directions without being root.
ARCHIVES="${APT_ARCHIVE_DIR:-/var/cache/apt/archives}"

# `Keep-Downloaded-Packages` is the one that makes the cache possible at all:
# apt has DELETED the `.deb` after a successful install since 1.5, so without
# it the save step below finds an empty directory and every run is a cold run.
# Nothing in a shimmed test can notice that, which is why the control asserts
# the option is passed rather than trusting the files to be there.
APT_OPTS="-o Acquire::Retries=3 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30 -o APT::Keep-Downloaded-Packages=true"

restored=0
if compgen -G "$CACHE/*.deb" > /dev/null 2>&1; then
    sudo mkdir -p "$ARCHIVES"
    sudo cp "$CACHE"/*.deb "$ARCHIVES"/ || true
    restored=$(find "$CACHE" -name '*.deb' | wc -l)
    echo "restored $restored cached package(s) into apt's archive directory"
else
    echo "no package cache yet; this run pays for the download"
fi

# Save whatever apt ended up with, cached or freshly fetched, so the next run
# starts warm. `apt-get install` leaves the archives in place unless something
# ran `clean`, and a restored file that was already used is simply copied back
# unchanged.
#
# ON THE WAY OUT, WHICHEVER WAY THAT IS. This ran only after a successful
# install until 19 August, which is exactly backwards: the run that fails is the
# one whose partial download is worth keeping, and a cache that fills only on
# success cannot help the failure it exists to prevent. Two CI runs in a row got
# worse that way - 16 jobs, then 18, every one of them stalled in this step -
# because each failure left the cache empty for the next.
#
# A half-fetched archive is safe to keep: apt checksums what it reuses and
# re-downloads anything that does not match.
# Drop cached `.deb` files the archive no longer offers.
#
# The cache key rolls only when the package list or the runner image changes, so
# without this a superseded version stays in the cache indefinitely and every run
# restores it, hands it to apt and watches apt ignore it. Harmless per file and
# unbounded over time, and it hides the real state: a cache full of versions
# nobody can fetch looks exactly like a warm cache.
#
# Keyed on what apt asks for AFTER a fresh index, since that is the only list
# that reflects the archive rather than the image.
prune_stale_cache() {
    compgen -G "$CACHE/*.deb" > /dev/null 2>&1 || return 0
    timeout $(( $(remaining) / 4 )) sudo apt-get update $APT_OPTS >/dev/null 2>&1 || return 0
    wanted="$(timeout 60 sudo apt-get install -y --no-install-recommends --print-uris $APT_OPTS $PACKAGES 2>/dev/null \
        | grep "^'" | sed "s|.*/||; s|'.*||" || true)"
    [ -n "$wanted" ] || return 0
    dropped=0
    for deb in "$CACHE"/*.deb; do
        name="$(basename "$deb")"
        printf '%s\n' "$wanted" | grep -qxF "$name" && continue
        # Keep anything already INSTALLED and merely absent from the fresh
        # resolution: apt does not ask again for what is on the machine, and
        # dropping those would cold-start the next run for no reason.
        dpkg-query -W -f='${Status}' "${name%%_*}" 2>/dev/null | grep -q "install ok installed" && continue
        # BOTH copies, or this does nothing. The restore step already put this
        # file into apt's archive directory, and `save_cache` copies that
        # directory back on the way out, so dropping only the cached one hands it
        # straight back to the next run.
        rm -f "$deb"
        sudo rm -f "$ARCHIVES/$name"
        dropped=$((dropped + 1))
    done
    [ "$dropped" -gt 0 ] && echo "dropped $dropped cached package(s) the archive no longer offers"
    return 0
}

save_cache() {
    mkdir -p "$CACHE"
    if compgen -G "$ARCHIVES/*.deb" > /dev/null 2>&1; then
        sudo cp -n "$ARCHIVES"/*.deb "$CACHE"/ 2>/dev/null || true
        sudo chown -R "$(id -u):$(id -g)" "$CACHE" 2>/dev/null || true
    fi
    echo "package cache holds $(find "$CACHE" -name '*.deb' 2>/dev/null | wc -l) file(s)"
}
trap save_cache EXIT

# THE SCRIPT ENDS BEFORE THE WORKFLOW ENDS IT. Three attempts of 240 + 420
# seconds is a budget of up to 33 minutes against a step ceiling of 10, so every
# stall was killed from outside, mid-download - and a step killed from outside
# runs no trap, saves no archive, and leaves the next run exactly as cold. That
# is the ratchet: 16 failing jobs, then 18.
#
# So the budget is the script's own, and it fits inside the ceiling with room for
# the save. Each attempt gets what is left rather than a fixed slice, and an
# attempt is only started if there is enough time for it to mean anything.
BUDGET="${APT_BUDGET_SECS:-450}"
deadline=$(( $(date +%s) + BUDGET ))
remaining() { echo $(( deadline - $(date +%s) )); }

# shellcheck disable=SC2086
ok=0
attempt=0

# A WARM RUN DOES NOT NEED THE MIRROR, AND UNTIL NOW IT ASKED ANYWAY. On
# 19 August one job failed with the cache working perfectly: 74 MB restored from
# an exact key, no package body fetched, and all three attempts still died - in
# `apt-get update`, the last line printed being an InRelease fetch, then eighty
# seconds of nothing. The `&&` below is why that was fatal. The index fetch ran
# first and the install never got to discover it needed nothing.
#
# So when the cache gave us packages, try installing them before asking the
# mirror anything. Every `.deb` is already in apt's archive directory and the
# runner image ships an index, so the common case resolves locally and the
# network is off the critical path entirely rather than merely off the large
# half of it.
#
# The staleness the retry loop exists for is unchanged: if the image's index is
# too old, or missing, or names a version that was pruned, this install fails -
# quickly, because failing to resolve a package is not a download - and the loop
# below runs `update` and does it properly. This can cost a few seconds; it
# cannot cost correctness, because apt either has what it needs or says so.
if [ "$restored" -gt 0 ] && [ "$(remaining)" -gt 60 ]; then
    # WHAT IS STILL MISSING, NAMED. On 19 August a job failed with the cache
    # HITTING - 174 packages restored - and then six minutes passing before the
    # budget ran out, which rules out the download this cache was built to avoid
    # and leaves "something else went to the mirror". `--print-uris` resolves the
    # install and prints what it WOULD fetch without fetching any of it, so the
    # next red run says whether the answer is one uncached package or an index,
    # instead of leaving it to be inferred from a timestamp.
    #
    # Resolved once and reused: a second resolution would be a second thing that
    # can be slow in the step whose slowness is under investigation. Timed and
    # non-fatal, because instrumentation that can fail the step it is diagnosing
    # is worse than none.
    wants="$(timeout 60 sudo apt-get install -y --no-install-recommends --print-uris $APT_OPTS $PACKAGES 2>/dev/null \
        | grep "^'" || true)"
    echo "after the restore, apt still wants $(printf '%s' "$wants" | grep -c . || true) file(s) from the mirror"
    printf '%s' "$wants" | head -5 | sed 's/^/  wants: /'

    echo "cache is warm; trying the install without touching the mirror"
    attempt_log="$(mktemp)"
    if timeout $(( $(remaining) - 30 )) sudo apt-get install -y --no-install-recommends $APT_OPTS $PACKAGES 2>&1 | tee "$attempt_log"
    then
        ok=1
    else
        # A 404 IS NOT A SLOW MIRROR, AND SAYING SO MATTERS. The other failures
        # here are a mirror that hangs or refuses; this one is the runner image's
        # apt index naming a version the archive has superseded, e.g.
        # `libheif-dev_1.17.6-1ubuntu4.6_amd64.deb 404 Not Found` on 21 August.
        # It is not transient and no number of retries against the same index
        # will fix it, so it is named as what it is and the index is refreshed
        # rather than retried into.
        if grep -q "404  *Not Found" "$attempt_log"; then
            echo "the index names a version the archive no longer has (404), so it is stale, not slow" >&2
            # And drop the cached copies of what the index no longer wants, so a
            # dead version cannot ride along in the saved cache for every future
            # run. `--print-uris` names exactly the files the CURRENT resolution
            # asks for; anything else in the cache is from a resolution that no
            # longer happens.
            prune_stale_cache
        else
            echo "the image's index was not enough; falling back to update" >&2
        fi
    fi
    rm -f "$attempt_log"
fi

# Both bounds, because they catch different failures: the budget is for a mirror
# that hangs, the attempt count for one that refuses instantly - without it a
# fast, repeatable "no" would spin for the whole budget, retrying something that
# is not going to change.
while [ "$ok" != 1 ] && [ "$attempt" -lt 3 ] && [ "$(remaining)" -gt 60 ]; do
    attempt=$((attempt + 1))
    left=$(remaining)
    # A third of what is left for the index, the rest for the packages: the
    # install is the long half and the update is worthless on its own.
    if timeout $(( left / 3 )) sudo apt-get update $APT_OPTS && \
       timeout $(( left - left / 3 - 5 )) sudo apt-get install -y --no-install-recommends $APT_OPTS $PACKAGES
    then
        ok=1; break
    fi
    echo "apt attempt $attempt stalled or failed, $(remaining)s of budget left; retrying" >&2
    # A moment before trying again, because the failure this retries is usually a
    # mirror that is briefly unhappy rather than one that is refusing. The pause
    # is overridable ONLY so the control can exercise the retry COUNT - which is
    # what the loop is about - without spending the wall clock waiting: three
    # failures took fifteen seconds per case there, and the control has several.
    # Unset means five, so CI and a real run are unchanged.
    sleep "${APT_RETRY_SLEEP:-5}"
done
# Which bound stopped us, because the two mean different things to whoever reads
# the red: three fast refusals is a mirror saying no, and a budget running out is
# a mirror that never answered.
[ "$ok" = 1 ] || {
    if [ "$attempt" -ge 3 ]; then
        echo "apt failed after three attempts" >&2
    else
        echo "apt ran out of its ${BUDGET}s budget after $attempt attempt(s)" >&2
    fi
    exit 1
}
