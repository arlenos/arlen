#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# What is an app's window actually called, in a given language?
#
# WHY THIS EXISTS AND A SCREENSHOT DOES NOT ANSWER IT. Every app sets
# `<svelte:head><title>` to its translated name, but that is the DOCUMENT title
# and it never leaves the webview. The name the topbar and the workspace
# overview show is the NATIVE window title, which comes from tauri.conf.json -
# so the catalog said "Dateien" while every surface outside the window said
# "Files", and no screenshot OF the app could show it, because the app draws no
# titlebar of its own (`decorations: false`). The only honest witness is the
# window manager, so this asks it.
#
# The app runs on its own Xvfb with a config directory it cannot escape, so the
# machine's real ~/.config is neither read nor written.
#
# Run: dev/screenshot/window-title.sh <app-binary> [locale] [expected]
#   dev/screenshot/window-title.sh target/release/arlen-clock-app de Uhr
# With `expected` it asserts and exits non-zero on a mismatch; without, it
# prints what it found.
#
# Build with `tauri build --no-bundle`; a plain `cargo build --release` leaves
# the binary pointing at devUrl, and that window is named by the dev server.
set -uo pipefail
app="${1:?usage: window-title.sh <app-binary> [locale] [expected]}"
loc="${2:-de}"
want="${3:-}"

[ -x "$app" ] || { echo "no binary at $app" >&2; exit 2; }
for bin in xvfb-run xdotool; do
  command -v "$bin" >/dev/null || { echo "error: '$bin' is needed to read a window name" >&2; exit 2; }
done

cfg=$(mktemp -d)
trap 'rm -rf "$cfg"' EXIT
mkdir -p "$cfg/arlen"
printf '[locale]\nui = "%s"\n' "$loc" > "$cfg/arlen/locale.toml"

title=$(xvfb-run -a --server-args="-screen 0 1280x900x24" bash -c '
  set -uo pipefail
  export XDG_CONFIG_HOME="'"$cfg"'"
  # A Wayland socket in the environment would send the app to the real session
  # instead of this display.
  unset WAYLAND_DISPLAY
  "'"$app"'" >/dev/null 2>&1 &
  pid=$!
  found=""
  # The title is set after the webview boots and adopts the language, so this
  # polls rather than sampling once.
  for _ in $(seq 1 40); do
    sleep 1
    for id in $(xdotool search --name "." 2>/dev/null); do
      n=$(xdotool getwindowname "$id" 2>/dev/null)
      case "$n" in ""|"Desktop") ;; *) found="$n";; esac
    done
    [ -n "$found" ] && break
  done
  kill $pid 2>/dev/null
  printf "%s" "$found"
')

if [ -z "$title" ]; then
  echo "no window appeared in 40s - the app did not start, or it drew nothing" >&2
  exit 1
fi
if [ -n "$want" ]; then
  if [ "$title" = "$want" ]; then
    echo "ok   window is named '$title' under $loc"
  else
    echo "FAIL window is named '$title' under $loc, expected '$want'" >&2
    exit 1
  fi
else
  echo "window title under $loc: $title"
fi
