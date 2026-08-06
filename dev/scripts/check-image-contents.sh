#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Ask the built image what is actually in it, without booting it.
#
# Every question below has been answered the expensive way at least once: build
# the image, boot it, watch something fail, work back to a missing file. A unit
# whose ExecStart does not exist fails at spawn with a bare "No such file or
# directory"; an app that was never installed is simply not there when you go
# looking for it. Both are visible in the raw file in about a minute.
#
#   dev/scripts/check-image-contents.sh [path/to/arlen.raw]
#
# Exits non-zero only for the unambiguous defect: a systemd unit that names an
# arlen binary the image does not ship. The rest is printed as inventory,
# because "which apps belong in the image" is a build decision and a script
# should not quietly hold an opinion about it.
set -uo pipefail

img="${1:-$(dirname "$0")/../mkosi/arlen.raw}"
if [ ! -f "$img" ]; then
  echo "no image at $img - build it first (dev/mkosi/build-image.sh)" >&2
  exit 2
fi

# `guestfish -i` does not recognise this image's layout ("no operating system
# was found"), so mount the root partition by hand. Doing it in one guestfish
# invocation keeps it to a single appliance boot; splitting the checks across
# calls costs ~20s each.
out=$(guestfish --ro -a "$img" run : mount-ro /dev/sda2 / : sh '
  echo "=== units naming a missing binary"
  for u in /usr/lib/systemd/system/*.service /usr/lib/systemd/user/*.service /etc/systemd/system/*.service; do
    [ -f "$u" ] || continue
    grep -hoE "^ExecStart=[+!-]*[^ ]+" "$u" 2>/dev/null | sed "s/^ExecStart=[+!-]*//" | while read -r b; do
      case "$b" in /*) [ -e "$b" ] || echo "$b <- $(basename "$u")";; esac
    done
  done | sort -u

  echo "=== arlen binaries shipped"
  ls /usr/bin 2>/dev/null | grep "^arlen" | sed "s|^|/usr/bin/|"
  ls /usr/lib/arlen/libexec 2>/dev/null | sed "s|^|/usr/lib/arlen/libexec/|"

  echo "=== desktop entries"
  ls /usr/share/applications/*.desktop 2>/dev/null | wc -l

  echo "=== accessibility bus"
  if [ -e /usr/libexec/at-spi-bus-launcher ] || [ -e /usr/lib/at-spi2-core/at-spi-bus-launcher ]; then
    echo present
  else
    echo "absent (a GTK app registers with nothing, so every AT-SPI query answers not-found)"
  fi
' 2>&1)

if [ -z "$out" ]; then
  echo "guestfish produced no output - the image did not mount, this is not a pass" >&2
  exit 2
fi
echo "$out"

# Stock Debian units ship conditional or alternative binaries: rc-local is
# guarded by ConditionFileIsExecutable, dbus.service names dbus-daemon while the
# image runs dbus-broker, and the quota units are inert without the quota tools.
# None of those are ours and none of them are a defect here.
missing=$(echo "$out" | sed -n '/=== units naming a missing binary/,/^=== /p' \
  | grep -E "^/" | grep -E "arlen" || true)

echo
if [ -n "$missing" ]; then
  echo "a unit names an arlen binary the image does not ship:"
  echo "$missing" | sed 's/^/  /'
  exit 1
fi
echo "no unit names a missing arlen binary"
