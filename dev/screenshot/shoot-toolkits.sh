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
#                 tarball unpacked); without it GTK3 renders stock Adwaita and the
#                 sheet's libadwaita names reach only GTK4.
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

# 1. The toolkit files, exactly as apply.rs writes them for this theme.
(cd "$root/sdk/theme" && cargo run -q --example emit -- "$variant" "$work/config") || exit 1
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
gtk3_theme="Adwaita"
if [ -n "${ADW_GTK3_DIR:-}" ] && [ -d "$ADW_GTK3_DIR/adw-gtk3" ]; then
  mkdir -p "$work/data/themes"
  cp -r "$ADW_GTK3_DIR/adw-gtk3" "$ADW_GTK3_DIR/adw-gtk3-dark" "$work/data/themes/"
  gtk3_theme="adw-gtk3"
  [ "$variant" = dark ] && gtk3_theme="adw-gtk3-dark"
elif [ "$variant" = dark ]; then
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
  printf 'exec env %s GDK_BACKEND=wayland GTK_THEME=%s gedit --new-window >%q 2>&1\n' \
    "$env_common" "$gtk3_theme" "$work/gtk3.log"
  printf 'exec env %s QT_QPA_PLATFORM=wayland QT_QPA_PLATFORMTHEME=qt6ct python3 %q >%q 2>&1\n' \
    "$env_common" "$here/toolkits/gallery-qt.py" "$work/qt6.log"
  # A GTK4 app that never linked libadwaita reads the theme_* names, so it is
  # the test of that half of the sheet; without a sound server it shows its
  # connection dialog, which is a window all the same.
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
echo "shot rc=$rc -> $out (gtk3 theme: $gtk3_theme, scheme: $scheme)"
exit $rc
