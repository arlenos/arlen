#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Put a real GTK window and a real Qt window beside an Arlen window and look.
#
# The colour mapping in sdk/theme/src/gtk.rs and qt.rs decides which Arlen
# colour lands on which toolkit role; that is a design decision, and until this
# script existed nobody had rendered a foreign app under it. So: emit the
# toolkit files for a bundled theme into a private XDG config dir, point GTK3
# (adw-gtk3), GTK4/libadwaita and Qt6 (qt6ct, Fusion) at them, and start the
# lot in one headless sway beside our own mail window, then grim the screen.
#
# The question the picture answers is not "are the colours ours" - they are by
# construction - but whether a foreign window beside ours looks like it belongs.
#
# Usage: dev/screenshot/shoot-toolkits.sh [dark|light] [out.png]
#   ADW_GTK3_DIR  a directory holding adw-gtk3/ and adw-gtk3-dark/ (the release
#                 tarball unpacked). Since the fork landed this is no longer how
#                 the GTK3 leg gets its shape: with it unset the script builds
#                 `themes/adw-gtk3` into the private data dir and renders the
#                 Arlen theme, which is the thing we actually ship. Set it only
#                 to compare against upstream's.
# Requires: sway, grim, python3 with PyGObject (Gtk 4 + Adw) and PyQt6, qt6ct.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
variant="${1:-dark}"
out="${2:-$here/out/foreign-toolkits-$variant.png}"
arlen_app="${ARLEN_APP:-$root/target/release/arlen-mail-app}"

work="$(mktemp -d "${TMPDIR:-/tmp}/arlen-toolkits.XXXXXX")"
# A fresh private runtime dir, never the session's: the headless sway socket
# must not sit beside the real compositor's, or grim can grab the real desktop.
export XDG_RUNTIME_DIR="$work/rt"
mkdir -p "$XDG_RUNTIME_DIR" "$work/config" "$work/data" "$work/cache"
chmod 700 "$XDG_RUNTIME_DIR"
cleanup() {
  [ -n "${sway_pid:-}" ] && kill "$sway_pid" 2>/dev/null
  [ -n "${sway_pid:-}" ] && wait "$sway_pid" 2>/dev/null
  rm -rf "$work"
  return 0
}
trap cleanup EXIT

# 1a. The GTK3 widget theme, from the vendored fork, into the private data dir.
# It goes first because the settings.ini apply.rs writes NAMES a theme only when
# one is installed, and it looks in this data dir - so building after emitting
# would leave the sheet unselected and the whole GTK3 leg would render stock.
gtk3_built=""
if [ -z "${ADW_GTK3_DIR:-}" ]; then
  mkdir -p "$work/data/themes"
  if XDG_DATA_HOME="$work/data" "$root/dev/scripts/build-gtk3-theme.sh" "$work/data/themes" \
      >"$work/gtk3-theme-build.log" 2>&1; then
    gtk3_built="Arlen"
  else
    echo "!! the GTK3 theme did not build, so its leg renders stock Adwaita:" >&2
    tail -3 "$work/gtk3-theme-build.log" >&2
  fi
fi

# 1b. The toolkit files, exactly as apply.rs writes them for this theme. The
# private data dir is passed through so the theme detection in the settings.ini
# sees what 1a just built rather than whatever is installed on the host.
(cd "$root/sdk/theme" && env XDG_DATA_HOME="$work/data" \
  cargo run -q --example emit -- "$variant" "$work/config") || exit 1
# qt6ct needs to be told to use the scheme; apply.rs deliberately leaves that
# choice to the user (their qt6ct.conf may carry other settings), so here the
# harness makes it.
mkdir -p "$work/config/qt6ct"
cat > "$work/config/qt6ct/qt6ct.conf" <<EOF
[Appearance]
style=Fusion
custom_palette=true
color_scheme_path=$work/config/qt6ct/colors/arlen.conf
standard_dialogs=default

[Fonts]
fixed="JetBrains Mono,10,-1,5,400,0,0,0,0,0,0,0,0,0,0,1"
general="Inter,10,-1,5,400,0,0,0,0,0,0,0,0,0,0,1"
EOF
# adw-gtk3, when supplied, as the GTK3 theme that reads the libadwaita names.
# Which theme the GTK3 window ends up in. Ours needs no override at all: the
# settings.ini emitted above names it, and letting GTK read that file is the
# point - it renders the path we ship rather than one the harness forced with
# `GTK_THEME=`, which is a debug variable no user has set.
gtk3_theme=""
if [ -n "${ADW_GTK3_DIR:-}" ] && [ -d "$ADW_GTK3_DIR/adw-gtk3" ]; then
  mkdir -p "$work/data/themes"
  cp -r "$ADW_GTK3_DIR/adw-gtk3" "$ADW_GTK3_DIR/adw-gtk3-dark" "$work/data/themes/"
  gtk3_theme="adw-gtk3"
  [ "$variant" = dark ] && gtk3_theme="adw-gtk3-dark"
elif [ -z "$gtk3_built" ] && [ "$variant" = dark ]; then
  gtk3_theme="Adwaita:dark"
fi
scheme="prefer-light"
[ "$variant" = dark ] && scheme="prefer-dark"

# 2. One sway, every window. Each exec carries the private XDG dirs and the
# toolkit switches; sway tiles them, which is exactly the side-by-side wanted.
env_common="XDG_CONFIG_HOME=$work/config XDG_DATA_HOME=$work/data XDG_CACHE_HOME=$work/cache"
cfg="$work/sway.cfg"
{
  echo "output HEADLESS-1 resolution 2400x1080"
  echo "default_border pixel 1"
  echo "font pango:Inter 10"
  # Ours first, on the left.
  if [ -x "$arlen_app" ]; then
    printf 'exec env %s GDK_BACKEND=wayland %q >%q 2>&1\n' "$env_common" "$arlen_app" "$work/arlen.log"
  fi
  printf 'exec env %s GDK_BACKEND=wayland ADW_DEBUG_COLOR_SCHEME=%s python3 %q >%q 2>&1\n' \
    "$env_common" "$scheme" "$here/toolkits/gallery-gtk.py" "$work/gtk4.log"
  if [ -n "$gtk3_theme" ]; then
    printf 'exec env %s GDK_BACKEND=wayland GTK_THEME=%s gedit --new-window >%q 2>&1\n' \
      "$env_common" "$gtk3_theme" "$work/gtk3.log"
  else
    printf 'exec env %s GDK_BACKEND=wayland gedit --new-window >%q 2>&1\n' \
      "$env_common" "$work/gtk3.log"
  fi
  printf 'exec env %s QT_QPA_PLATFORM=wayland QT_QPA_PLATFORMTHEME=qt6ct python3 %q >%q 2>&1\n' \
    "$env_common" "$here/toolkits/gallery-qt.py" "$work/qt6.log"
  # A GTK4 app that never linked libadwaita, and the measurement it produced
  # refutes the reason it was put here. The comment used to say such an app
  # "reads the theme_* names, so it is the test of that half of the sheet"; on
  # 8 September its window came out #353535, which is GTK4's own Adwaita dark,
  # while the libadwaita gallery beside it came out our #0f0f0f exactly. So the
  # dark SCHEME reaches a plain GTK4 app through settings.ini and the PALETTE
  # does not: GTK4's built-in stylesheet does not resolve those names from a
  # user sheet the way GTK3's Adwaita does. It is still the test of that half,
  # and the answer is no. Without a sound server it shows its connection
  # dialog, which is a window all the same.
  if command -v pavucontrol >/dev/null 2>&1; then
    printf 'exec env %s GDK_BACKEND=wayland pavucontrol >%q 2>&1\n' "$env_common" "$work/gtk4plain.log"
  fi
} > "$cfg"

WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 sway -c "$cfg" >"$work/sway.log" 2>&1 &
sway_pid=$!
sleep 18
wd="$(ls "$XDG_RUNTIME_DIR" 2>/dev/null | grep -E '^wayland-[0-9]+$' | head -1)"
if [ -z "$wd" ]; then
  echo "!! no headless sway socket; refusing to grab" >&2
  cat "$work/sway.log" >&2
  exit 1
fi
mkdir -p "$(dirname "$out")"
WAYLAND_DISPLAY="$wd" grim "$out"; rc=$?
for l in arlen gtk4 gtk3 qt6 gtk4plain; do
  if [ -s "$work/$l.log" ]; then echo "-- $l.log:"; tail -3 "$work/$l.log"; fi
done
echo "shot rc=$rc -> $out (gtk3: ${gtk3_theme:-${gtk3_built:-stock Adwaita} via settings.ini}, scheme: $scheme)"
exit $rc
