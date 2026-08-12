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
#
# Naming an image and being asked about a file that is not there is an error;
# naming none when none is built is not. That distinction is what lets this run
# from `just check-executor` without either lying or failing on a tree that has
# never built an image, and it was added on 11 Aug when the check turned out to
# be the one file in dev/scripts that nothing invoked. It had sat there correct
# and unread, which is the shape the standing rule exists to catch: a check
# nobody runs cannot be told apart from a check that passes.
set -uo pipefail

explicit=1
img="${1:-}"
if [ -z "$img" ]; then
  explicit=0
  img="$(dirname "$0")/../mkosi/arlen.raw"
fi

if [ ! -f "$img" ]; then
  if [ "$explicit" = 1 ]; then
    echo "no image at $img" >&2
    exit 2
  fi
  echo "no image built at $img; nothing to inspect (dev/mkosi/build-image.sh builds one)"
  exit 0
fi

# Same reasoning for the tool: an image is present and we cannot open it, so say
# which of the two it is rather than reporting the image as clean.
if ! command -v guestfish >/dev/null 2>&1; then
  echo "an image is present at $img but guestfish is not installed, so it was NOT inspected" >&2
  echo "  (Debian/Ubuntu: libguestfs-tools, Arch: libguestfs)" >&2
  exit "$([ "$explicit" = 1 ] && echo 2 || echo 0)"
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

  echo "=== desktop entries naming a missing binary"
  for d in /usr/share/applications/*.desktop; do
    [ -f "$d" ] || continue
    e=$(grep -m1 "^Exec=" "$d" 2>/dev/null | sed "s/^Exec=//; s/ .*//")
    [ -n "$e" ] || continue
    case "$e" in
      /*) [ -e "$e" ] || echo "$e <- $(basename "$d")" ;;
      *)  [ -e "/usr/bin/$e" ] || [ -e "/usr/local/bin/$e" ] || echo "$e <- $(basename "$d")" ;;
    esac
  done

  echo "=== shipped arlen units not enabled"
  for u in /usr/lib/systemd/user/arlen-*.service /usr/lib/systemd/system/arlen-*.service; do
    [ -f "$u" ] || continue
    n=$(basename "$u")
    find /etc/systemd /usr/lib/systemd -name "$n" -type l 2>/dev/null | grep -q . || echo "$n"
  done

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

# ...and non-empty is not the same as answered. The script above runs INSIDE the
# guest, so an image whose `/bin/sh` is missing - a changed layout, the wrong
# partition mounted at sda2, a minimal rootfs - makes every command produce
# nothing while guestfish's own error keeps `out` non-empty. The emptiness guard
# then passes and every conclusion below is drawn over an image that was never
# read: "no unit names a missing arlen binary" about a filesystem nobody looked at.
#
# Measured 12 Aug against a fixture image with no shell in it, which reported a
# clean bill of health. Bound by STRUCTURE rather than by length: each section the
# inner script prints must be present, because that is what "it ran" looks like.
for marker in "=== units naming a missing binary" "=== arlen binaries shipped" \
              "=== desktop entries" "=== desktop entries naming a missing binary" \
              "=== shipped arlen units not enabled" "=== accessibility bus"; do
  case "$out" in
    *"$marker"*) ;;
    *)
      echo "the inspection did not run: no '$marker' section in the output." >&2
      echo "guestfish reported:" >&2
      echo "$out" >&2
      exit 2
      ;;
  esac
done
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

# Desktop entries were an open question until it was settled that the image ships
# them: the shell's app index already parses /usr/share/applications, apps
# arriving via apt or forage bring their own, and discovering first-party apps by
# some other route would be a second store of the same facts. So zero entries
# alongside shipped app binaries is now a defect rather than a curiosity.
entries=$(echo "$out" | sed -n '/=== desktop entries/,/^=== /p' | grep -E '^[0-9]+$' | head -1)
apps_shipped=$(echo "$img_bins" | grep -cE '^arlen-(files|terminal|meetings|system-monitor)$' || true)

echo
if [ "${entries:-0}" -eq 0 ] && [ "${apps_shipped:-0}" -gt 0 ]; then
  echo "the image ships $apps_shipped app binaries and no desktop entries, so the launcher"
  echo "  enumerates nothing - rebuild after staging them (apps/*/dist/*.desktop)"
  exit 1
fi
echo "desktop entries present: ${entries:-0}"

# The confined launcher, reported rather than failed on. `shell.toml [launcher]
# confined` defaults false, so its absence costs nothing today - but the flag
# names this binary on PATH, so flipping it without the binary makes every launch
# fail to spawn. Read the name out of the shell rather than repeating it here, so
# a rename cannot leave this checking for a launcher nobody runs.
launcher=$(grep -hoE 'const LAUNCHER: &str = "[^"]+"' \
  "$repo_root/apps/desktop-shell/core/src/launch/plan.rs" 2>/dev/null | sed 's/.*"\(.*\)"/\1/')
echo
if [ -z "$launcher" ]; then
  echo "could not read the launcher name from the shell; skipped that check"
elif echo "$img_bins" | grep -qx "$launcher"; then
  echo "confined launcher present: $launcher"
else
  echo "confined launcher ABSENT: the shell runs \`$launcher\` on PATH when"
  echo "  [launcher] confined = true, and the image ships no such binary, so the"
  echo "  flip would make every launch fail to spawn. Not an error while the flag"
  echo "  defaults off; it is what has to be built before the flag means anything."
fi

# The user-facing twin of the unit check above. A `.desktop` entry is what the
# launcher lists, so an Exec that is not on the image is an icon the user clicks
# and nothing happens - no error they can read, because the failure is a spawn
# that never happened. The section above this one counts the entries; counting
# them says nothing about whether they work, and the count is exactly what would
# stay reassuring while an app dropped out of the build.
dead_entries=$(echo "$out" | sed -n '/=== desktop entries naming a missing binary/,/^=== /p' \
  | grep -E "<- .*\.desktop$" || true)

echo
if [ -n "$dead_entries" ]; then
  echo "a desktop entry names a binary the image does not ship:"
  echo "$dead_entries" | sed 's/^/  /'
  echo "  The launcher lists these, so each is an icon that does nothing when clicked."
  exit 1
fi
echo "every desktop entry names a binary the image ships"

# A unit that ships and is never enabled is installed-but-not-started, which the
# image cannot tell you apart from working. This reads the ARTEFACT rather than
# the mechanism on purpose: the enable comes from mkosi's preset pass acting on
# each unit's `[Install]`, and the tree ALSO carries hand-kept `.wants` symlinks
# that duplicate it. On 13 Aug those two disagreed by one entry - the undo signer
# was absent from the committed set - so reading the tree said it never ran while
# the image had it enabled and the boot had it running. Only the built image
# settles that, and the same read would catch the opposite (a unit that lost its
# `[Install]` and quietly stopped being enabled), which no source-side check can.
unenabled=$(echo "$out" | sed -n '/=== shipped arlen units not enabled/,/^=== /p' \
  | grep -E "^arlen-.*\.service$" || true)

echo
if [ -n "$unenabled" ]; then
  echo "a shipped arlen unit is enabled nowhere on the image, so it never starts:"
  echo "$unenabled" | sed 's/^/  /'
  echo "  Every shipped unit carries an [Install] section and mkosi's preset pass"
  echo "  enables it; a unit missing here has lost that stanza or is not being"
  echo "  presetted, and installed is not started."
  exit 1
fi
echo "every shipped arlen unit is enabled"

echo
if [ -n "$missing" ]; then
  echo "a unit names an arlen binary the image does not ship:"
  echo "$missing" | sed 's/^/  /'
  exit 1
fi
echo "no unit names a missing arlen binary"
