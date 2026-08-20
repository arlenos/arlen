#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Open every app the image ships, on the image, and keep the picture.
#
# WHY THIS EXISTS. On 20 August I opened all eleven by hand, one boot at a time,
# and four of them were wrong in ways nothing else could show: the clock was
# refused by its own healthy daemon and said "cannot read your saved clock data";
# the terminal's curated zsh config was installed on a machine with no zsh, so
# the block UI it is built around was inert; the text editor's lens had a heading
# over nothing where the graph is real; the screenshot app turned out to be fine
# and my harness was clicking it. Each one passes its unit tests, its drives on
# the developer host, and every gate in `dev/scripts`.
#
# The difference is always the same: on this host a daemon is usually absent, a
# tool is usually installed, and a store is usually empty in a different way than
# it is on the machine. So the sweep has to happen where the machine is, and it
# should be one command rather than an afternoon.
#
# THE APP LIST COMES FROM THE IMAGE, not from a list here: every `Exec=` in
# `/usr/share/applications`. A list in this file would drift the first time
# something is staged, and drift silently, since nobody would notice an app that
# stopped being photographed.
#
# What this does NOT do is grade the frames. A picture is for a person to look
# at; the checks that can be automated live in `dev/screenshot/drive-*.sh`, and
# what is left here is exactly the part that needs eyes.
#
# Run: dev/screenshot/shoot-image-apps.sh [path-to-arlen.raw]
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
image="${1:-$root/dev/mkosi/arlen.raw}"
out="$here/out"

[ -f "$image" ] || { echo "no image at $image - build it with dev/mkosi/build-image.sh"; exit 2; }
command -v guestfish >/dev/null 2>&1 || { echo "guestfish is needed to read the app list off the image"; exit 2; }

echo "reading the app list off $image"
apps=$(guestfish --ro -a "$image" run : mount-ro /dev/sda2 / : sh \
  'grep -h "^Exec=" /usr/share/applications/*.desktop 2>/dev/null | sed "s/^Exec=//;s/ .*//" | sort -u' \
  2>/dev/null | tr -d '\r')

[ -n "$apps" ] || { echo "the image ships no desktop entries, so there is nothing to open"; exit 2; }

count=$(printf '%s\n' "$apps" | grep -c .)
echo "$count app(s) to open; each is a boot, so this takes a while"
echo

for app in $apps; do
  shot="$out/image-$app.png"
  printf '%-24s ' "$app"
  # `--approve-consent` because the dogfood consent card sits over the middle of
  # every unapproved frame on this image, and the app frame is the one worth
  # keeping. Its own after-shot is what carries the app.
  log=$(timeout 600 python3 "$root/dev/vm/verify.py" --image "$image" --wait 100 \
        --approve-consent --app "$app" --out "$shot" 2>&1)
  if printf '%s' "$log" | grep -q "not an installed binary"; then
    echo "not installed"
  elif printf '%s' "$log" | grep -q "no launch signal"; then
    echo "no launch signal - see the frame"
  elif [ -s "$shot.approved.png" ]; then
    echo "-> $(basename "$shot").approved.png"
  else
    echo "no frame captured"
  fi
done

echo
echo "the frames are in $out. Look at them: that is the whole point of this script."
