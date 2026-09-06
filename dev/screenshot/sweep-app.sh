#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Sweep ONE app's routes for text that came out wrong, server and all.
#
# WHY IT EXISTS. `sweep-render.sh` takes a base URL and says "start the app's dev
# server first", so every run of it starts a server by hand - and a server started
# by hand is a server nobody stops. Measured on 6 September, after a night of
# rendering: six `vite dev` processes were still listening, on 6431, 6441, 6451,
# 6461, 6471 and 6811, hours after the renders that started them. None of those
# ports belongs to any app; they were invented per run, which is exactly why
# nothing could account for them afterwards.
#
# THE LEAK IS NOT THE COST. A left-over server answers the next `curl`, and
# `curl` cannot say who answered. On 7 September a dev server abandoned on 6421
# answered a readiness check I believed was my own `vite preview` - which had
# quietly taken the next free port - so two renders came back in German from a
# server I had not started. Everything I read was true and none of it was of the
# page I thought I was looking at.
#
# So this owns the server: it takes the app's OWN dev port (from `tauri.conf.json`,
# which `check-dev-ports.py` already pins to the vite config), starts it through
# the shared helper - which refuses rather than testing a port it did not start -
# and stops it again, verifying the port went quiet.
#
# It must be `vite dev`, not a preview: the kit's `applyDevLocale` is gated on
# `import.meta.env.DEV`, so a preview renders the source language whatever
# `?locale=` says. That is the one thing this wrapper must not get wrong, and it
# is why `start_dev` exists.
#
# Run: dev/screenshot/sweep-app.sh knowledge de / "/::#tab-library"
# Every argument after the app name is passed to sweep-render.sh untouched.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
# shellcheck source=dev/screenshot/lib/wait.sh
. "$here/lib/wait.sh"
# shellcheck source=dev/screenshot/lib/preview.sh
. "$here/lib/preview.sh"

app="${1:?usage: sweep-app.sh <app> <locale> <path> [path...]}"
shift
[ $# -ge 2 ] || { echo "give a locale and at least one path" >&2; exit 2; }

dir="$root/apps/$app"
[ -d "$dir" ] || { echo "no app at $dir" >&2; exit 2; }

# The port the app itself declares. Inventing one here is how the strays above
# happened, and a port an app does not own tells you nothing about that app.
conf="$dir/src-tauri/tauri.conf.json"
[ -f "$conf" ] || { echo "no tauri.conf.json under $dir, so no dev port to read" >&2; exit 2; }
port="$(sed -n 's/.*"devUrl": *"http:\/\/localhost:\([0-9]*\)".*/\1/p' "$conf" | head -1)"
[ -n "$port" ] || { echo "$conf names no devUrl port" >&2; exit 2; }

PREVIEW_PGID=""
cleanup() { stop_preview; return 0; }
trap cleanup EXIT

start_dev "$dir" "$port" || exit 2
wait_for_http "http://localhost:$port/" || exit 2

"$here/sweep-render.sh" "http://localhost:$port" "$@"
