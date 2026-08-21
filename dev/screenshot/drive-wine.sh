#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Open the Windows-programs window on a bottle that has been tampered with, and
# read what it says about it.
#
# WHY THE FIXTURE HAS A DOOR NOBODY OPENED. A bottle's whole claim is "this
# program reaches only the folders you granted it", and Wine itself contradicts
# that claim on the day it creates a prefix: `dosdevices/z:` points at `/`. So
# the fixture below is a prefix with exactly that in it and one honest grant, and
# the load-bearing assertion is that the window NAMES the ungranted letter rather
# than listing the grant and looking tidy. A window that shows only what the
# description claims is the defect this app exists to prevent.
#
# The empty case matters for a different reason: a person with no bottles needs
# somewhere to go, not a definition. Every other app on this image names its
# folder, and this asserts that this one does too.
#
# Run: dev/screenshot/drive-wine.sh [path-to-arlen-wine-manager-app]
#
# Build with `tauri build --no-bundle`, not a plain `cargo build`.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
app="${1:-$root/target/release/arlen-wine-manager-app}"
fix="$HOME/.cache/arlen-drive-wine"
fail=0

[ -x "$app" ] || { echo "no wine-manager binary at $app"; exit 2; }
rm -rf "$fix"; mkdir -p "$fix"

say() {  # say <name> <ok> <detail>
  if [ "$2" = 1 ]; then echo "  ok   $1"; else echo "  FAIL $1"; echo "       $3"; fail=1; fi
}

cat > "$fix/probe.js" <<'JS'
await new Promise((r) => setTimeout(r, 2500));
return document.body.innerText.replace(/\s+/g, " ").trim().slice(0, 1200);
JS

echo "wine-manager:"

# 1. No bottles. The window has to say where one would live, not only what one is.
empty="$fix/empty"
mkdir -p "$empty"
got=$(SHOOT_APP_ENV="XDG_DATA_HOME=$empty" SHOOT_INJECT="$fix/probe.js" \
  "$here/shoot-app.sh" "$app" "$here/out/wine-empty.png" 2>&1 \
  | sed -n 's/^inject result: //p')
say "with no bottles it says what one is" \
  "$(printf '%s' "$got" | grep -q "No bottles yet" && echo 1 || echo 0)" "$got"
# The path is the part a person can act on. Written with the isolate marks the
# formatter puts around an interpolated value, so this looks for the tail.
say "and where one would be kept" \
  "$(printf '%s' "$got" | grep -q "arlen/bottles" && echo 1 || echo 0)" "$got"

# 2. A bottle with one honest grant and Wine's own door to the whole disk.
data="$fix/data"
prefix="$fix/prefix"
mkdir -p "$data/arlen/bottles/notepad" "$prefix/dosdevices" "$prefix/drive_c" "$fix/docs"
ln -sfn "$fix/docs" "$prefix/dosdevices/d:"
ln -sfn ../drive_c "$prefix/dosdevices/c:"
ln -sfn / "$prefix/dosdevices/z:"
cat > "$data/arlen/bottles/notepad/bottle.toml" <<TOML
id = "notepad"
prefix_root = "$prefix"
egress = "none"

[[grants]]
host = "$fix/docs"
access = "read_only"
TOML

got=$(SHOOT_APP_ENV="XDG_DATA_HOME=$data" SHOOT_INJECT="$fix/probe.js" \
  "$here/shoot-app.sh" "$app" "$here/out/wine-bottle.png" 2>&1 \
  | sed -n 's/^inject result: //p')

say "a granted folder is shown with its letter and what it may do" \
  "$(printf '%s' "$got" | grep -q "D:" \
     && printf '%s' "$got" | grep -q "docs" \
     && printf '%s' "$got" | grep -q "read only" && echo 1 || echo 0)" "$got"

# THE case. Z: is not in the description and reaches everything.
say "the door nobody granted is named, not left out of the list" \
  "$(printf '%s' "$got" | grep -q "never granted" \
     && printf '%s' "$got" | grep -q "z:" && echo 1 || echo 0)" "$got"

say "and the window offers to close it" \
  "$(printf '%s' "$got" | grep -q "Put this bottle back" && echo 1 || echo 0)" "$got"

# A bottle with no network says so rather than saying nothing.
say "a bottle with no network says so" \
  "$(printf '%s' "$got" | grep -q "not allowed" && echo 1 || echo 0)" "$got"

# 3. German, because a person reading this in German is reading about their own
# disk and the words have to be theirs.
cfg="$fix/config-de"
mkdir -p "$cfg/arlen"
printf '[locale]\nui = "de"\n' > "$cfg/arlen/locale.toml"
got=$(XDG_DATA_HOME="$data" XDG_CONFIG_HOME="$cfg" SHOOT_INJECT="$fix/probe.js" \
  "$here/shoot-app.sh" "$app" "$here/out/wine-bottle-de.png" 2>&1 \
  | sed -n 's/^inject result: //p')
say "the German build says the German words" \
  "$(printf '%s' "$got" | grep -q "nie freigegeben" && echo 1 || echo 0)" "$got"

# The bottle's own id comes from the file and stays as written.
say "and leaves the bottle's own name alone" \
  "$(printf '%s' "$got" | grep -q "notepad" && echo 1 || echo 0)" "$got"

[ "$fail" = 0 ] && echo "the window shows the doors that are open, not the ones the description claims"
exit "$fail"
