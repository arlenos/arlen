#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Put real foreign windows beside an Arlen window and look.
#
# The colour mapping in sdk/theme/src/gtk.rs, qt.rs and wine.rs decides which
# Arlen colour lands on which toolkit role; that is a design decision, and until
# this script existed nobody had rendered a foreign app under it. So: emit the
# toolkit files for a bundled theme into a private XDG config dir, point each
# toolkit at them, and start a group of windows in one headless sway beside our
# own mail window, then grim the screen.
#
# The question the picture answers is not "are the colours ours" - they are by
# construction - but whether a foreign window beside ours looks like it belongs.
#
# Usage: dev/screenshot/shoot-toolkits.sh [dark|light] [gtk|qt|wine|all] [out-dir]
#   gtk   gedit (GTK3, on the Arlen theme built from themes/adw-gtk3 when `sass`
#         is on PATH, else stock Adwaita), pavucontrol (GTK4 without libadwaita,
#         the theme_* names) and a libadwaita gallery.
#   qt    the same Qt Widgets gallery under Qt6/qt6ct and Qt5/qt5ct.
#   wine  a fresh prefix with the theme's .reg imported, winecfg and notepad
#         under Wine's Wayland driver.
# Requires: sway, grim, python3 with PyGObject (Gtk 4 + Adw), PyQt6 and PyQt5,
# qt6ct, qt5ct, wine; sass for the GTK3 shape.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
variant="${1:-dark}"
group="${2:-all}"
outdir="${3:-$here/out}"
arlen_app="${ARLEN_APP:-$root/target/release/arlen-mail-app}"

work="$(mktemp -d "${TMPDIR:-/tmp}/arlen-toolkits.XXXXXX")"
# A fresh private runtime dir, never the session's: the headless sway socket
# must not sit beside the real compositor's, or grim can grab the real desktop.
export XDG_RUNTIME_DIR="$work/rt"
mkdir -p "$XDG_RUNTIME_DIR" "$work/config" "$work/data/themes" "$work/cache"
chmod 700 "$XDG_RUNTIME_DIR"
sway_pid=""
cleanup() {
  [ -n "$sway_pid" ] && kill "$sway_pid" 2>/dev/null && wait "$sway_pid" 2>/dev/null
  [ -d "$work/pfx" ] && WINEPREFIX="$work/pfx" wineserver -k >/dev/null 2>&1
  rm -rf "$work"
  return 0
}
trap cleanup EXIT

# 1. The toolkit files, exactly as apply.rs writes them for this theme, plus
# the Wine document the bottle daemon would import.
# `ARLEN_TOOLKIT_CUSTOMIZATION=<theme.toml>` layers a customization over the
# bundled theme before the toolkit files are written, which is how a PICKED
# colour gets into this picture. Without it the gallery can only ever show what
# ships, and "does the accent somebody chose reach a GTK button" is the question
# the mapping most needs answered.
(cd "$root/sdk/theme" && cargo run -q --example emit -- \
  "$variant" "$work/config" ${ARLEN_TOOLKIT_CUSTOMIZATION:+"$ARLEN_TOOLKIT_CUSTOMIZATION"} >/dev/null) || exit 1
# qt6ct and qt5ct need to be told to use the scheme and the house font;
# apply.rs deliberately leaves both to the user (their qtNct.conf may carry
# other settings), so here the harness makes the choice.
for q in qt6ct qt5ct; do
  mkdir -p "$work/config/$q"
  cat > "$work/config/$q/$q.conf" <<EOF
[Appearance]
style=Fusion
custom_palette=true
color_scheme_path=$work/config/$q/colors/arlen.conf
standard_dialogs=default

[Fonts]
fixed="JetBrains Mono,10,-1,5,400,0,0,0,0,0,0,0,0,0,0,1"
general="Inter,10,-1,5,400,0,0,0,0,0,0,0,0,0,0,1"
EOF
done
# The GTK3 shape: the Arlen theme built out of the vendored adw-gtk3 fork. It
# reads the same @define-color names as libadwaita, so colour arrives at
# runtime; the build is shape only and cheap.
gtk3_theme="Adwaita"
[ "$variant" = dark ] && gtk3_theme="Adwaita:dark"
if command -v sass >/dev/null 2>&1; then
  if "$root/dev/scripts/build-gtk3-theme.sh" "$work/data/themes" >"$work/gtk3-build.log" 2>&1; then
    gtk3_theme="Arlen"
    [ "$variant" = dark ] && gtk3_theme="Arlen:dark"
  else
    echo "!! the GTK3 theme did not build; GTK3 renders stock Adwaita:" >&2
    tail -5 "$work/gtk3-build.log" >&2
  fi
else
  echo "!! no sass on PATH; GTK3 renders stock Adwaita, not the Arlen shape" >&2
fi
scheme="prefer-light"
[ "$variant" = dark ] && scheme="prefer-dark"

env_common="XDG_CONFIG_HOME=$work/config XDG_DATA_HOME=$work/data XDG_CACHE_HOME=$work/cache"

# 2. A Wine prefix with the theme imported, when the wine group is wanted.
# Booted here rather than taken from the person's own: the point is the palette
# and nothing else, and a throwaway prefix carries nothing else.
wine_ready=0
prepare_wine() {
  command -v wine >/dev/null 2>&1 || { echo "!! no wine on PATH" >&2; return 1; }
  export WINEPREFIX="$work/pfx" WINEDEBUG=-all WINEDLLOVERRIDES="mscoree,mshtml="
  wineboot -i >"$work/wineboot.log" 2>&1 || { echo "!! wineboot failed" >&2; tail -3 "$work/wineboot.log" >&2; return 1; }
  wine regedit /S "$(winepath -w "$work/config/wine.reg")" >"$work/regedit.log" 2>&1 \
    || { echo "!! regedit refused the theme document" >&2; return 1; }
  wineserver -w
  wine_ready=1
}

# 3. One sway per group, every window of the group in it. Each exec carries the
# private XDG dirs and the toolkit switches; sway tiles them, which is exactly
# the side-by-side wanted.
shoot_group() {
  local g="$1"
  local out="$outdir/foreign-toolkits-$variant-$g.png" cfg="$work/sway-$g.cfg"
  {
    echo "output HEADLESS-1 resolution 2400x1080"
    # No sway chrome in the picture: a floating window (winecfg is one) would
    # otherwise get sway's blue title bar, and a focused window its blue frame,
    # both of which read as the toolkit's until you know better.
    echo "default_border pixel 1"
    echo "default_floating_border pixel 1"
    echo "client.focused #27272a #27272a #fafafa #27272a #27272a"
    echo "client.unfocused #27272a #27272a #a1a1aa #27272a #27272a"
    echo "client.urgent #27272a #27272a #fafafa #27272a #27272a"
    echo "font pango:Inter 10"
    # Ours first, on the left.
    if [ -x "$arlen_app" ]; then
      printf 'exec env %s GDK_BACKEND=wayland %q >%q 2>&1\n' "$env_common" "$arlen_app" "$work/arlen.log"
    fi
    case "$g" in
      gtk)
        printf 'exec env %s GDK_BACKEND=wayland GTK_THEME=%s gedit --new-window >%q 2>&1\n' \
          "$env_common" "$gtk3_theme" "$work/gtk3.log"
        if command -v pavucontrol >/dev/null 2>&1; then
          printf 'exec env %s GDK_BACKEND=wayland pavucontrol >%q 2>&1\n' "$env_common" "$work/gtk4plain.log"
        fi
        printf 'exec env %s GDK_BACKEND=wayland ADW_DEBUG_COLOR_SCHEME=%s python3 %q >%q 2>&1\n' \
          "$env_common" "$scheme" "$here/toolkits/gallery-gtk.py" "$work/gtk4.log"
        ;;
      qt)
        printf 'exec env %s QT_QPA_PLATFORM=wayland QT_QPA_PLATFORMTHEME=qt6ct python3 %q >%q 2>&1\n' \
          "$env_common" "$here/toolkits/gallery-qt.py" "$work/qt6.log"
        printf 'exec env %s QT_GALLERY_MAJOR=5 QT_QPA_PLATFORM=wayland QT_QPA_PLATFORMTHEME=qt5ct python3 %q >%q 2>&1\n' \
          "$env_common" "$here/toolkits/gallery-qt.py" "$work/qt5.log"
        ;;
      wine)
        # No DISPLAY in this session, so Wine picks its Wayland driver.
        printf 'exec env %s WINEPREFIX=%q WINEDEBUG=-all wine winecfg >%q 2>&1\n' \
          "$env_common" "$work/pfx" "$work/winecfg.log"
        printf 'exec env %s WINEPREFIX=%q WINEDEBUG=-all wine notepad >%q 2>&1\n' \
          "$env_common" "$work/pfx" "$work/notepad.log"
        ;;
    esac
  } > "$cfg"

  WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 sway -c "$cfg" >"$work/sway-$g.log" 2>&1 &
  sway_pid=$!
  sleep 20
  local wd
  wd="$(ls "$XDG_RUNTIME_DIR" 2>/dev/null | grep -E '^wayland-[0-9]+$' | head -1)"
  if [ -z "$wd" ]; then
    echo "!! no headless sway socket for $g; refusing to grab" >&2
    cat "$work/sway-$g.log" >&2
    return 1
  fi
  mkdir -p "$outdir"
  WAYLAND_DISPLAY="$wd" grim "$out"; local rc=$?
  kill "$sway_pid" 2>/dev/null; wait "$sway_pid" 2>/dev/null; sway_pid=""
  for l in arlen gtk4 gtk3 gtk4plain qt6 qt5 winecfg notepad; do
    if [ -s "$work/$l.log" ]; then echo "-- $l.log:"; tail -2 "$work/$l.log"; fi
  done
  echo "shot rc=$rc -> $out"
  return $rc
}

rc=0
case "$group" in
  gtk|qt) shoot_group "$group" || rc=1 ;;
  wine) prepare_wine && shoot_group wine || rc=1 ;;
  all)
    shoot_group gtk || rc=1
    shoot_group qt || rc=1
    prepare_wine && shoot_group wine || rc=1
    ;;
  *) echo "usage: shoot-toolkits.sh [dark|light] [gtk|qt|wine|all] [out-dir]" >&2; exit 2 ;;
esac
echo "gtk3 theme: $gtk3_theme, scheme: $scheme, wine: $wine_ready"
exit $rc
