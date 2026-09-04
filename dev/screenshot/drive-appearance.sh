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
# The frontend is served from a preview below, not baked into the binary, so the
# staleness guard compares Rust only (see shoot-app.sh).
export SHOOT_FRONTEND_SERVED=1
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

# One metric of each declared type. The risk on this path is a TYPE: a numeric
# field written as the wrong kind of number makes theme.toml unparsable, and an
# unparsable customization file does not lose one row, it takes the whole theme
# down. A font weight went out as `800.0` for a `u32` field and the resolver
# refused the entire file - which this drive is how we found out.
cat > "$work/metrics.js" <<'JS'
const inv = window.__TAURI_INTERNALS__.invoke;
return inv("theme_set_metric", { key: "radius.card", value: "18" })
  .then(() => inv("theme_set_metric", { key: "depth.blur_enabled", value: "false" }))
  .then(() => inv("theme_set_metric", { key: "spacing.md", value: "0.9rem" }))
  .then(() => inv("theme_set_metric", { key: "typography.weight_bold", value: "800" }))
  .then(() => inv("theme_resolved_metrics"))
  .then((m) => JSON.stringify({
    card: m["radius.card"], blur: m["depth.blur_enabled"],
    md: m["spacing.md"], bold: m["typography.weight_bold"],
  }))
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
# Coordinates track the ANSI strip's position; the sound section moving to its
# own page (24 Aug) lifted the preview column, so the row sits higher now.
red, green = im.getpixel((322, 280)), im.getpixel((360, 280))
if red != (255, 0, 85):
    raise SystemExit(f"!! the preview's red swatch is {red}, not the overridden #ff0055")
if green != (22, 163, 74):
    raise SystemExit(f"!! the neighbouring green moved to {green}; an override must be slot-exact")
print(f">> preview: red {red} is the override, green {green} is untouched")
PY

echo ">> launch 3: write one metric of each type and resolve the file again"
SHOOT_INJECT="$work/metrics.js" SHOOT_INJECT_SETTLE=3 \
  "$root/dev/screenshot/shoot-app.sh" "$app" "" "" 8 \
  | tee "$work/run3.log" | grep -E "inject result" || true

# The read is `theme_resolved_metrics`, which RESOLVES the file. A type the schema
# refuses shows up as `error: resolve: ...` rather than as a wrong value, so the
# assertion is that all four came back at all.
grep -q '"bold":"800"' "$work/run3.log" \
  || { echo "!! the metrics did not survive resolution - a type is wrong" >&2; exit 1; }
grep -q '"blur":"false"' "$work/run3.log" || { echo "!! the boolean metric did not take" >&2; exit 1; }
grep -q 'weight_bold = 800$' "$XDG_CONFIG_HOME/arlen/theme.toml" \
  || { echo "!! the weight is not a whole number in the file" >&2; exit 1; }

# Clearing is the half that fails SILENTLY. `config_reset` does not throw when it
# deletes nothing, so a wrong path leaves the row reset on screen and the override
# in the file, and the value returns at the next launch - which reads as a reset
# button that works sometimes.
cat > "$work/clear.js" <<'JS'
const inv = window.__TAURI_INTERNALS__.invoke;
return inv("theme_set_system", { key: "ansi1", value: null })
  .then(() => inv("theme_set_color", { role: "accent", hex: null }))
  .then(() => Promise.all([inv("theme_system_overrides"), inv("theme_color_overrides")]))
  .then(([sys, col]) => JSON.stringify({ ansi1: sys.ansi1 ?? "cleared", accent: col.accent ?? "cleared" }))
  .catch((e) => JSON.stringify({ error: String(e) }));
JS

echo ">> launch 4: clear both and check the file, not just the answer"
SHOOT_INJECT="$work/clear.js" SHOOT_INJECT_SETTLE=3 \
  "$root/dev/screenshot/shoot-app.sh" "$app" "" "" 8 \
  | tee "$work/run4.log" | grep -E "inject result" || true

grep -q '"ansi1":"cleared"' "$work/run4.log" || { echo "!! the ANSI slot did not clear" >&2; exit 1; }
grep -q '"accent":"cleared"' "$work/run4.log" || { echo "!! the accent did not clear" >&2; exit 1; }
# `if`, not `grep && exit`: under `set -e` a failing left-hand side of `&&` does
# not abort, so that form works only by a shell rule subtle enough to misread as
# an assertion that never fires.
if grep -q '#ff0055' "$XDG_CONFIG_HOME/arlen/theme.toml"; then
    echo "!! the file still holds the override the page says is gone" >&2
    exit 1
fi
if grep -q '#00ddaa' "$XDG_CONFIG_HOME/arlen/theme.toml"; then
    echo "!! the file still holds the accent the page says is gone" >&2
    exit 1
fi

echo "PASS: an appearance edit reaches the file, survives the process, renders and clears"
