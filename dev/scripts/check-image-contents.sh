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

# The staging step has now fallen behind the build step three times - fifteen
# units, the settings binary, every desktop entry. The pattern is that an
# artifact's real home is next to its crate and the image carries a
# hand-maintained copy, so anything added crate-side has to be added image-side
# too and nothing notices when it is not.
#
# A blunt "every repo unit must be staged" rule would be wrong: most unstaged
# units belong to daemons the image deliberately does not ship, and they are
# absent consistently. The defect worth failing on is the INCONSISTENT case - the
# image ships the binary but not the unit that starts it, so the daemon is
# present and can never run.
repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
img_bins=$(echo "$out" | sed -n '/=== arlen binaries shipped/,/^=== /p' | sed 's|.*/||' | grep -E '^arlen' || true)
staged=$(ls "$repo_root"/dev/mkosi/mkosi.extra/usr/lib/systemd/system \
            "$repo_root"/dev/mkosi/mkosi.extra/usr/lib/systemd/user \
            "$repo_root"/dev/mkosi/mkosi.extra/usr/share/dbus-1/services 2>/dev/null | sort -u)

orphans=""
while read -r unit; do
  [ -n "$unit" ] || continue
  base=$(basename "$unit")
  echo "$staged" | grep -qx "$base" && continue
  exec_line=$(grep -hoE '^(ExecStart|Exec)=[+!-]*[^ ]+' "$unit" 2>/dev/null | head -1 | sed 's/^[A-Za-z]*=[+!-]*//')
  [ -n "$exec_line" ] || continue
  echo "$img_bins" | grep -qx "$(basename "$exec_line")" || continue
  orphans="$orphans  $base (starts $(basename "$exec_line"), which the image ships)\n"
done <<EOF
$(cd "$repo_root" && git ls-files '*.service' | grep -vE 'mkosi|node_modules')
EOF

echo
if [ -n "$orphans" ]; then
  echo "the image ships these binaries but not the unit that starts them:"
  printf "%b" "$orphans"
  exit 1
fi
echo "every shipped arlen binary has its unit staged"

echo
if [ -n "$missing" ]; then
  echo "a unit names an arlen binary the image does not ship:"
  echo "$missing" | sed 's/^/  /'
  exit 1
fi
echo "no unit names a missing arlen binary"
