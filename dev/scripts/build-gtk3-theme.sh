#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Build the Arlen GTK3 widget theme out of the vendored adw-gtk3 fork.
#
# What this produces is a theme DIRECTORY - `<out>/Arlen/gtk-3.0/gtk.css` plus
# the dark sheet, the symbolic assets and an `index.theme` - which is the thing
# GTK3 selects by name. That is a different layer from the `@define-color`
# overrides `sdk/theme` writes into `~/.config/gtk-3.0/gtk.css`: this one carries
# the SHAPE (radius, spacing, the flat style) and the override file carries the
# COLOUR. The compiled sheet references GTK named colours rather than baking
# hexes, so the two compose and a palette change needs no rebuild.
#
# The shape comes in through `_arlen-tokens.scss`, which is written into a build
# tree rather than checked in - a checked-in copy of numbers that live in the
# theme is a second source that drifts.
#
#   dev/scripts/build-gtk3-theme.sh <out-dir> [tokens.scss]
#
# With no tokens file it writes the house defaults below, which is what a
# development build wants; the caller that has a resolved theme passes its own.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
fork="$here/themes/adw-gtk3"
out="${1:?usage: build-gtk3-theme.sh <out-dir> [tokens.scss]}"
tokens="${2:-}"

if ! command -v sass >/dev/null 2>&1; then
  echo "build-gtk3-theme: no \`sass\` on PATH, so the theme cannot be built." >&2
  echo "  Install dart-sass (arch: \`sass\`, fedora: \`dart-sass\`)." >&2
  exit 2
fi

build="$(mktemp -d)"
trap 'rm -rf "$build"' EXIT
cp -r "$fork/src/sass" "$build/sass"

if [ -n "$tokens" ]; then
  cp "$tokens" "$build/sass/_arlen-tokens.scss"
else
  # The house defaults, stated once. A caller with a resolved ArlenTheme passes
  # its own file and these are not consulted.
  cat > "$build/sass/_arlen-tokens.scss" <<'TOKENS'
// Written by the build. Do not check a copy of this in.
$button_radius: 8px;
$menu_radius: 8px;
$window_radius: 12px;
$popover_radius: 12px;
$card_radius: 12px;
$dialog_radius: 12px;
$check_radius: 4px;
TOKENS
fi

theme="$out/Arlen/gtk-3.0"
mkdir -p "$theme"
sass --no-source-map "$build/sass/arlen.scss" "$theme/gtk.css"
sass --no-source-map "$build/sass/arlen-dark.scss" "$theme/gtk-dark.css"
cp -r "$fork/src/assets" "$theme/assets"
sed -e 's/@VariantThemeName@/Arlen/g' "$fork/src/index.theme.in" > "$out/Arlen/index.theme"

echo "built $theme/gtk.css ($(wc -c <"$theme/gtk.css") bytes) and gtk-dark.css"
