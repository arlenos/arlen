#!/bin/bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Drive Settings > Appearance and prove an edit SURVIVES the app.
#
# WHAT THIS ANSWERS THAT A UNIT TEST CANNOT. Until 19 Aug the whole Appearance
# override system was a set of controls attached to variables that died with the
# window: the stores held edits in memory, the one write command that existed
# persisted into a table the theme loader never read, and the pages opened on
# defaults every time. Every piece of that had its own green test. What nobody had
# done was change something, close the app, open it again, and look.
#
# So this runs the app TWICE. The first launch writes through the real commands
# the controls call; the second is a fresh process that must find them. A store
# that only seemed to work because the value was still in memory fails on the
# second launch, which is the failure mode the unit tests structurally cannot see.
#
# The config lives in a temp XDG_CONFIG_HOME, so a drive never edits the config of
# whoever ran it. That is not politeness: the whole point is reading what is on
# disk afterwards, and doing that in a real home means the second run starts from
# whatever the first one left in the developer's own theme.
#
# Run: dev/screenshot/drive-appearance.sh [out-dir]
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
out="${1:-$root/dev/screenshot/out}"
app="$root/target/debug/arlen-settings"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

[ -x "$app" ] || { echo "!! build it first: cargo build --manifest-path apps/settings/src-tauri/Cargo.toml" >&2; exit 1; }

export XDG_CONFIG_HOME="$work/config"
mkdir -p "$XDG_CONFIG_HOME/arlen"

# The router is SvelteKit's, so navigation is a link CLICK. A raw pushState leaves
# the page where it was and prints a cheerful "navigated" beside a screenshot of
# the old route, which is how the first attempt at this passed while proving
# nothing.
cat > "$work/goto.js" <<'JS'
const a = [...document.querySelectorAll("a[href]")].find((n) => n.getAttribute("href").endsWith("/appearance/system"));
if (!a) return JSON.stringify({ went: false });
a.click();
return JSON.stringify({ went: true });
JS

cat > "$work/write.js" <<'JS'
const inv = window.__TAURI_INTERNALS__.invoke;
return inv("theme_set_system", { key: "ansi1", value: "#ff0055" })
  .then(() => inv("theme_set_color", { role: "accent", hex: "#00ddaa" }))
  .then(() => JSON.stringify({ wrote: true }))
  .catch((e) => JSON.stringify({ wrote: false, error: String(e) }));
JS

cat > "$work/read.js" <<'JS'
const inv = window.__TAURI_INTERNALS__.invoke;
return Promise.all([inv("theme_system_overrides"), inv("theme_color_overrides")])
  .then(([sys, col]) => JSON.stringify({ ansi1: sys.ansi1, accent: col.accent }))
  .catch((e) => JSON.stringify({ error: String(e) }));
JS

echo ">> launch 1: write two overrides through the commands the controls call"
SHOOT_INJECT="$work/goto.js:$work/write.js" SHOOT_INJECT_SETTLE=3 \
  "$root/dev/screenshot/shoot-app.sh" "$app" "$out/appearance-system-written.png" "" 8 \
  | tee "$work/run1.log" | grep -E "inject result|wrote " || true

grep -q '"wrote":true' "$work/run1.log" || { echo "!! the write did not go through" >&2; exit 1; }

# On disk, before anything reads it back through the app: a read-back that only
# consults the app's own memory would pass with an empty file.
echo ">> theme.toml after the first launch"
sed 's/^/   /' "$XDG_CONFIG_HOME/arlen/theme.toml"
grep -q 'red = "#ff0055"' "$XDG_CONFIG_HOME/arlen/theme.toml" \
  || { echo "!! the ANSI slot is not in the file" >&2; exit 1; }
grep -q 'accent = "#00ddaa"' "$XDG_CONFIG_HOME/arlen/theme.toml" \
  || { echo "!! the accent is not in the file" >&2; exit 1; }

echo ">> launch 2: a fresh process must find them"
SHOOT_INJECT="$work/goto.js:$work/read.js" SHOOT_INJECT_SETTLE=3 \
  "$root/dev/screenshot/shoot-app.sh" "$app" "$out/appearance-system-persisted.png" "" 8 \
  | tee "$work/run2.log" | grep -E "inject result|wrote " || true

grep -q '"ansi1":"#ff0055"' "$work/run2.log" || { echo "!! the second launch did not find the ANSI slot" >&2; exit 1; }
grep -q '"accent":"#00ddaa"' "$work/run2.log" || { echo "!! the second launch did not find the accent" >&2; exit 1; }

# And rendered, not merely returned. The live preview draws the resolved palette,
# so the second swatch of the ANSI strip is the value that came back - a page that
# stored the override and drew the theme's own colour would pass every check above.
python3 - "$out/appearance-system-persisted.png" <<'PY'
import sys
from PIL import Image
im = Image.open(sys.argv[1]).convert("RGB")
red, green = im.getpixel((310, 285)), im.getpixel((350, 285))
if red != (255, 0, 85):
    raise SystemExit(f"!! the preview's red swatch is {red}, not the overridden #ff0055")
if green != (22, 163, 74):
    raise SystemExit(f"!! the neighbouring green moved to {green}; an override must be slot-exact")
print(f">> preview: red {red} is the override, green {green} is untouched")
PY

echo "PASS: an appearance edit reaches the file, survives the process and renders"
