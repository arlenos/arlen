#!/usr/bin/env bash
# Boot the image once per app and keep the frame its window is in.
#
# WHY THIS EXISTS. Three apps went onto the image - settings, harness, store -
# each with a build step, a binary, a desktop entry and an icon, and nothing had
# ever confirmed one of them OPENS. A build step proves a file was copied. The
# only thing that proves an app runs on this system is this system running it.
#
# It is a script rather than three commands in a report because the answer goes
# stale on every image: whoever asks "does the store still open" next month should
# press one thing, not reconstruct the invocation from a paragraph.
#
# Usage: dev/vm/verify-apps.sh [app ...]     (default: the three from the image)
#        IMAGE=/path/to.raw dev/vm/verify-apps.sh arlen-settings
#
# Each app boots the image with its binary name in the SMBIOS SKU, which
# `arlen-session` reads and launches after the shell. `--require-bar` fails the
# boot before the app is even reached if the desktop did not render, so a failure
# here is never ambiguous about which half broke.
#
# NB it does NOT assert on OCR, for two reasons and the first is decisive: this
# host has no `tesseract`, so `--require-app-text` cannot read anything here at
# all. `verify.py` is honest about that - its `ocr()` answers None, "I could not
# look", rather than an empty string, after that exact confusion made a text
# assert fail for an app that had drawn perfectly. The second reason holds even
# where tesseract is installed: under llvmpipe the OCR misses rendered text often
# enough that a red would say more about the software rasteriser than about the
# app. The frames are kept and meant to be LOOKED at; that is the whole point of
# taking them.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
img="${IMAGE:-$here/../mkosi/arlen.raw}"
[ -f "$img" ] || { echo "image not found: $img (build it first)"; exit 2; }
# A window has to come up after the shell does, so the deadline is past the
# bar-only one. It is a deadline, not a sleep: a boot returns when its bar renders.
WAIT="${WAIT:-120}"

apps=("$@")
[ ${#apps[@]} -gt 0 ] || apps=(arlen-settings arlen-harness arlen-store)

outdir="${OUTDIR:-$(mktemp -d /tmp/arlen-verify-apps.XXXXXX)}"
echo "== booting $img once per app, wait=${WAIT}s"
echo "== frames and serial logs in $outdir"

failed=0
for app in "${apps[@]}"; do
    shot="$outdir/$app.png"
    ser="$outdir/$app.serial.log"
    log="$outdir/$app.log"
    if python3 "$here/verify.py" --image "$img" --require-bar --wait "$WAIT" \
            --app "$app" --out "$shot" --serial-out "$ser" >"$log" 2>&1; then
        echo "$app: booted and launched -> $shot"
    else
        failed=$((failed + 1))
        echo "$app: FAILED (exit $?) -> $log"
        # The line worth having in front of somebody: the session says plainly
        # when the SKU named a binary it could not find.
        grep -m1 "verify app" "$ser" 2>/dev/null | sed 's/^/    session: /'
    fi
done

echo
if [ "$failed" -gt 0 ]; then
    echo "== $failed of ${#apps[@]} did not come up; the serial log says why"
    exit 1
fi
echo "== all ${#apps[@]} booted and launched. LOOK AT THE FRAMES:"
for app in "${apps[@]}"; do echo "   $outdir/$app.png"; done
