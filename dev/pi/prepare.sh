#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Build the pi engine Arlen runs: the pinned upstream tag plus the one patch in
# this directory.
#
# WHY THIS EXISTS. The image build (`05b-pi.sh.chroot`), `just pi-e2e` and the
# daemon all read a built pi at $ARLEN_PI_INSTALL, and until 11 November 2026
# the only description of how that tree came to be was a line in an error
# message: clone the tag, `npm ci`, build. That recipe produces an engine that
# cannot make a model call, because the proxy-socket patch lived in one detached
# commit of one clone on one machine and nowhere in this repository. A second
# machine could not have built the image's engine.
#
# Run: dev/pi/prepare.sh [--source-only]
#   --source-only   clone and patch, skip `npm ci` and the build
# The tree goes to $ARLEN_PI_INSTALL (default ~/Repositories/pi). An existing
# tree is refused, never overwritten: move it away yourself if you mean to.
# $ARLEN_PI_SOURCE names the repository to clone, for a mirror or a local copy.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PI_TAG="v0.79.10"
source_url="${ARLEN_PI_SOURCE:-https://github.com/earendil-works/pi.git}"
dest="${ARLEN_PI_INSTALL:-$HOME/Repositories/pi}"
source_only=0
[ "${1:-}" = "--source-only" ] && source_only=1

if [ -e "$dest" ]; then
    echo "refusing: $dest exists. This script builds a fresh tree and does not touch one that is there." >&2
    exit 2
fi

git -c advice.detachedHead=false clone --quiet --depth 1 --branch "$PI_TAG" "$source_url" "$dest"
git -C "$dest" apply --check "$here/arlen-proxy-socket.patch"
git -C "$dest" apply "$here/arlen-proxy-socket.patch"
echo ">> pi $PI_TAG cloned to $dest with the proxy-socket patch applied"
[ "$source_only" = 1 ] && exit 0

# `--ignore-scripts`: a dependency's install script is code from the network run
# with this user's rights, and the build needs none of them.
( cd "$dest" && npm ci --ignore-scripts --no-audit --no-fund --loglevel=error && npm run build )
[ -f "$dest/packages/coding-agent/dist/cli.js" ] || { echo "!! the build left no packages/coding-agent/dist/cli.js" >&2; exit 1; }
echo ">> built: $dest/packages/coding-agent/dist/cli.js"
