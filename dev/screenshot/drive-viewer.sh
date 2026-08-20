#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Drive the viewer through the behaviours `quickview-plan.md` names, and read
# what came back. Open, next/previous, zoom, rotate, delete-with-undo, audio.
#
# WHY A SCRIPT RATHER THAN A READ. On 18 August the board still listed the viewer
# as open work. Grepping the source for "zoom" and "rotate" finds them and proves
# nothing: it cannot tell a control that works from one that is wired to nothing,
# and it cannot see that the zoom label agrees with the picture. Pressing them
# can. Every claim this prints came from the running app answering.
#
# Fixtures are generated here rather than committed: the audio one has to be a
# real decodable file with an amplitude envelope (a flat tone draws a rectangle
# and would pass a waveform that ignored the file), and the images have to sit in
# $HOME, because the delete case moves a file to the home trash and a rename
# cannot cross a filesystem - see `contracts/freedesktop-trash`.
#
# Run: dev/screenshot/drive-viewer.sh [path-to-arlen-viewers]
#
# Build the binary with `tauri build --no-bundle`, NOT a plain `cargo build`: the
# latter leaves it pointing at devUrl and the run reports on whatever dev server
# holds that port. `shoot-app.sh` says so when it happens.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
app="${1:-$root/target/release/arlen-viewers}"
fix="$HOME/.cache/arlen-drive-viewer"
fail=0

[ -x "$app" ] || { echo "no viewer binary at $app"; exit 2; }
mkdir -p "$fix" "$here/out"

python3 - "$fix" <<'PY'
import math, os, struct, sys, wave
fix = sys.argv[1]
try:
    from PIL import Image, ImageDraw
except ImportError:
    sys.exit("PIL is needed to make the image fixtures")
# Landscape, portrait: the portrait one is what makes a rotate visible as a
# shape change rather than a redraw of the same rectangle.
for name, size, bg in (("a-one.png", (900, 600), (30, 90, 140)),
                       ("b-two.png", (900, 600), (90, 30, 140)),
                       ("c-portrait.png", (400, 900), (25, 25, 35))):
    im = Image.new("RGB", size, bg)
    d = ImageDraw.Draw(im)
    d.rectangle([40, 60, size[0] - 40, size[1] - 60], fill=(220, 180, 60))
    im.save(os.path.join(fix, name))
# Three seconds, stereo, with an |sin| envelope so the waveform has a shape only
# a decoder that read the samples could draw.
w = wave.open(os.path.join(fix, "tone.wav"), "wb")
w.setnchannels(2); w.setsampwidth(2); w.setframerate(44100)
w.writeframes(b"".join(
    struct.pack("<hh", v, v)
    for v in (int(20000 * (0.15 + 0.85 * abs(math.sin(i / 44100 * math.pi)))
                  * math.sin(2 * math.pi * 440 * i / 44100))
              for i in range(44100 * 3))))
w.close()
print(f"fixtures in {fix}")
PY

say() {
  local name="$1" ok="$2" got="$3"
  if [ "$ok" = 1 ]; then echo "  ok   $name"; else echo "  FAIL $name"; echo "       $got"; fail=1; fi
}

drive_bare() {  # drive_bare <probe-js> <out-png> - launched with NO file
  # What the launcher gives a person. Every drive in this directory hands its app
  # a file, so the empty state was never opened here - and that is how the viewer,
  # the editor and mail came to answer the same situation three different ways.
  printf '%s' "$(SHOOT_INJECT="$1" \
    "$here/shoot-app.sh" "$app" "$here/out/$2" 2>&1 \
    | sed -n 's/^inject result: //p')"
}

drive() {  # drive <probe-js> <fixture> <out-png>
  printf '%s' "$(SHOOT_APP_ARGS="$fix/$2" SHOOT_INJECT="$1" \
    "$here/shoot-app.sh" "$app" "$here/out/$3" 2>&1 \
    | sed -n 's/^inject result: //p')"
}

echo "viewer:"

# POLLED, not slept. This probe read the DOM the instant it was injected, which
# wins on a warm run and loses on a cold one: the first launch of the suite is
# the slowest, and a read before the first paint returns an empty body that looks
# exactly like an app which opened nothing. It failed that way on 19 August in a
# run whose other seven cases passed. Waiting FOR the content rather than for a
# guessed number of milliseconds is the difference between a case that is slow
# and a case that is wrong.
cat > "$fix/p-open.js" <<'JS'
for (let i = 0; i < 40; i++) {
  const level = document.querySelector(".level");
  if (level && level.textContent.trim()) break;
  await new Promise(r => setTimeout(r, 100));
}
return JSON.stringify({ dock: (document.querySelector(".level")||{}).textContent,
  body: (document.body.innerText||"").replace(/\s+/g," ").trim().slice(0,40) });
JS
got=$(drive "$fix/p-open.js" a-one.png viewer-open.png)
say "opens the file it was given, and says where it is in the folder" \
  "$(printf '%s' "$got" | grep -q "a-one.png 1 / 3" && echo 1 || echo 0)" "$got"

cat > "$fix/p-next.js" <<'JS'
const n = document.querySelector('[aria-label*="ext"]');
if (!n) return "no next control";
n.click(); await new Promise(r => setTimeout(r, 1200));
return (document.body.innerText||"").replace(/\s+/g," ").trim().slice(0,40);
JS
got=$(drive "$fix/p-next.js" a-one.png viewer-next.png)
say "next moves to the next file in the folder" \
  "$(printf '%s' "$got" | grep -q "b-two.png 2 / 3" && echo 1 || echo 0)" "$got"

# The label and the picture have to agree. They did not until 16 August: at fit
# the dock said "100%", which in every other viewer means one image pixel per
# screen pixel, and here means a multiple of the FITTED size.
cat > "$fix/p-zoom.js" <<'JS'
const face = () => (document.querySelector(".level")||{}).textContent?.trim();
const before = face();
const zin = document.querySelector('[aria-label*="Zoom in"], [aria-label*="ergr"]');
if (!zin) return "no zoom control";
zin.click(); await new Promise(r=>setTimeout(r,150));
zin.click(); await new Promise(r=>setTimeout(r,400));
const cv = document.querySelector("canvas");
return JSON.stringify({ before, after: face(),
  transform: cv ? getComputedStyle(cv).transform : null });
JS
got=$(drive "$fix/p-zoom.js" a-one.png viewer-zoom.png)
say "zoom moves the picture and the label agrees with it" \
  "$(printf '%s' "$got" | grep -q '"before":"Fit"' \
     && printf '%s' "$got" | grep -q '"after":"156%"' \
     && printf '%s' "$got" | grep -q "1.5625" && echo 1 || echo 0)" "$got"

cat > "$fix/p-rotate.js" <<'JS'
const cv = () => document.querySelector("canvas");
const before = cv() ? [cv().width, cv().height] : null;
window.dispatchEvent(new KeyboardEvent("keydown", { key: "r", bubbles: true }));
await new Promise(r => setTimeout(r, 700));
return JSON.stringify({ before, after: cv() ? [cv().width, cv().height] : null });
JS
got=$(drive "$fix/p-rotate.js" c-portrait.png viewer-rotate.png)
say "rotate turns the picture, not just the frame around it" \
  "$(printf '%s' "$got" | grep -q '\[400,900\]' \
     && printf '%s' "$got" | grep -q '\[900,400\]' && echo 1 || echo 0)" "$got"

# The half worth driving is the second one: a delete that cannot be taken back
# is a different feature from the one the plan names.
cat > "$fix/p-delete.js" <<'JS'
window.dispatchEvent(new KeyboardEvent("keydown", { key: "Delete", bubbles: true }));
await new Promise(r => setTimeout(r, 1500));
const err = document.querySelector('[role="alert"], .error');
const undo = [...document.querySelectorAll("button")]
  .find(b => /undo|rückgängig/i.test(b.textContent||""));
if (!undo) return JSON.stringify({ undo: "none offered",
  err: err && err.textContent.trim().slice(0,90) });
undo.click(); await new Promise(r => setTimeout(r, 1500));
return JSON.stringify({ undone: true, err: err && err.textContent.trim().slice(0,90) });
JS
got=$(drive "$fix/p-delete.js" b-two.png viewer-delete.png)
say "delete goes to the trash and undo brings the file back" \
  "$(printf '%s' "$got" | grep -q '"undone":true' && [ -f "$fix/b-two.png" ] && echo 1 || echo 0)" \
  "$got (file present afterwards: $([ -f "$fix/b-two.png" ] && echo yes || echo NO))"

# NB the wording here was "an audio file PLAYS", and it was wrong. There is no
# `<audio>` element, no `AudioContext` and no `new Audio()` anywhere in
# apps/viewers: `playing` in AudioPlayer.svelte is a `$state(true)` boolean that
# Space toggles and the transport draws an icon from. So the face renders and
# responds; it emits no sound, and a probe cannot assert otherwise because there
# is nothing to assert against.
#
# What IS real is checked: the waveform canvas, the stream details read off the
# file by ffprobe rather than invented (pcm_s16le at 44100 is the fixture this
# script generates), and the transport actually changing state on Space - which
# is the behaviour the component has.
cat > "$fix/p-audio.js" <<'JS'
await new Promise(r => setTimeout(r, 1600));
window.dispatchEvent(new KeyboardEvent("keydown", { key: "i", bubbles: true }));
await new Promise(r => setTimeout(r, 900));
const cv = document.querySelector("canvas");
// The transport's primary button names its NEXT action, so the label flips when
// the state does. Read it, press Space, read it again.
// BUTTONS only. The first cut scanned every `[aria-label]` and matched the
// player container's own "Audio player" on /play/, so before and after were the
// same string and the case failed against my own selector rather than the app.
const label = () => {
  const b = [...document.querySelectorAll("button[aria-label]")]
    .map(e => e.getAttribute("aria-label"))
    .filter(l => /^(pause|play|wiedergabe|pausieren)/i.test((l || "").trim()));
  return b[0] || null;
};
const before = label();
window.dispatchEvent(new KeyboardEvent("keydown", { key: " ", bubbles: true, cancelable: true }));
await new Promise(r => setTimeout(r, 500));
return JSON.stringify({ waveform: cv ? [cv.width, cv.height] : null,
  transportBefore: before, transportAfter: label(),
  details: (document.body.innerText||"").replace(/\s+/g," ").trim().slice(0,160) });
JS
got=$(drive "$fix/p-audio.js" tone.wav viewer-audio.png)
say "an audio file opens with a waveform and stream details read off the file itself" \
  "$(printf '%s' "$got" | grep -q '"waveform":\[' \
     && printf '%s' "$got" | grep -q "pcm_s16le" \
     && printf '%s' "$got" | grep -q "44100" && echo 1 || echo 0)" "$got"
# Separate case, because it fails for a different reason: the face can render
# correctly while its one interaction does nothing.
say "and Space moves the transport between play and pause" \
  "$(printf '%s' "$got" | python3 -c "
import json,sys
try:
    d = json.loads(sys.stdin.read() or '{}')
except Exception:
    print(0); raise SystemExit
b, a = d.get('transportBefore'), d.get('transportAfter')
print(1 if b and a and b != a else 0)")" "$got"

# Printing. The button is the FIRST caller the print portal has ever had: the
# backend could hand a document to CUPS since it was written and nothing in the
# system asked it to. Pressing it here goes to the real portal on this machine,
# so the run may flash a print dialog; the app is killed at the end of the shot,
# which drops the request.
cat > "$fix/p-print.js" <<'JS'
const b = document.querySelector('[aria-label="Print"], [aria-label="Drucken"]');
if (!b) return "no print control in the dock";
b.click();
await new Promise(r => setTimeout(r, 900));
return JSON.stringify({ status: (document.querySelector('[role="status"]')||{}).innerText,
  body: (document.body.innerText||"").replace(/\s+/g," ").trim().slice(0,120) });
JS
got=$(drive "$fix/p-print.js" a-one.png viewer-print.png)
# Pending, not printed: the portal answers when a person does, and claiming a
# document reached a printer before that is exactly the kind of statement this
# app is not allowed to make.
say "the print control hands the file to the portal and says the request is pending" \
  "$(printf '%s' "$got" | grep -qiE "print service|Druckdienst" && echo 1 || echo 0)" "$got"

cat > "$fix/p-bare.js" <<'JS'
await new Promise(r => setTimeout(r, 2000));
return document.body.innerText.replace(/\s+/g, " ").trim().slice(0, 200);
JS

bare=$(drive_bare "$fix/p-bare.js" viewer-no-file.png)

# OPENED WITH NOTHING, IT MUST SAY WHERE A FILE COMES FROM. This window cannot
# open one itself - it takes a path from `%f` or argv - so "No file is open." on
# its own tells a person nothing they can act on. And the demo track the browser
# preview shows must not be here: a shipped viewer once showed "Nightswim" with a
# playhead at 1:13 of 3:40, none of which exists.
say "launched with no file, it says where a file comes from" \
  "$(printf '%s' "$bare" | grep -qE "file manager|Dateiverwaltung" && echo 1 || echo 0)" "$bare"

say "and shows no invented track" \
  "$(printf '%s' "$bare" | grep -q "Nightswim" && echo 0 || echo 1)" "$bare"

[ "$fail" = 0 ] && echo "every behaviour the plan names answered when it was pressed, and an empty window that says what to do"
exit "$fail"
