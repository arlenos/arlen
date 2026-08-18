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
# WHAT THE CACHE ACTUALLY REMOVES, stated honestly: the package bodies, not the
# index. `apt-get update` still talks to the mirror, because a stale index is
# how you get a 404 on a version that was pruned. That fetch is metadata and a
# few MB; the 200 MB of `.deb` files is what was on the critical path, and on a
# warm cache apt finds every one of them already in `/var/cache/apt/archives`
# and downloads nothing. The retry around the whole call stays, because the
# index fetch can still be slow and a bounded attempt against a fresh mirror is
# the answer to that.
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
PACKAGES="
protobuf-compiler build-essential pkg-config cmake nasm
libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev
libssl-dev libdbus-1-dev fuse3 libfuse3-dev
libdav1d-dev libheif-dev zlib1g-dev libheif-plugin-libde265
"

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

# shellcheck disable=SC2086
ok=0
for attempt in 1 2 3; do
    if timeout 240 sudo apt-get update $APT_OPTS && \
       timeout 420 sudo apt-get install -y --no-install-recommends $APT_OPTS $PACKAGES
    then
        ok=1; break
    fi
    echo "apt attempt $attempt stalled or failed; retrying" >&2
    sleep 5
done
[ "$ok" = 1 ] || { echo "apt failed after three attempts" >&2; exit 1; }

# Save whatever apt ended up with, cached or freshly fetched, so the next run
# starts warm. `apt-get install` leaves the archives in place unless something
# ran `clean`, and a restored file that was already used is simply copied back
# unchanged.
mkdir -p "$CACHE"
if compgen -G "$ARCHIVES/*.deb" > /dev/null 2>&1; then
    sudo cp -n "$ARCHIVES"/*.deb "$CACHE"/ || true
    sudo chown -R "$(id -u):$(id -g)" "$CACHE"
fi
echo "package cache holds $(find "$CACHE" -name '*.deb' | wc -l) file(s)"
