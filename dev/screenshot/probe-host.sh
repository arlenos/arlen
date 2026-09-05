#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Run a render probe against a surface that is REFUSING something, and refuse to
# report anything unless the surface actually got there.
#
# WHY IT CHECKS FIRST. A host script in `hosts/` is a claim: install this runtime,
# drive this gesture, and the app reaches the state named in the filename. Nothing
# was checking the claim. `files-refuses-op` dispatched `contextmenu` on `main`
# for weeks - events bubble UP, and `main` is an ancestor of the menu's trigger -
# so it opened no menu, refused nothing, and every picture and every probe answer
# taken through it was about a plain folder listing. A clean answer from a page
# that never reached the state is the same false green the controls in
# `dev/scripts` exist to stop, one level up.
#
# So each host declares what its state says, in one line near the top:
#
#   // EXPECT: Das Dateisystem hat sich geweigert
#
# A host with no such line is a refusal, not a default: whoever writes the
# fixture knows what it should produce, and nobody else can guess it.
#
#   dev/screenshot/probe-host.sh files-refuses-op http://localhost:1427 \
#     dev/screenshot/clipped-text.js 720
set -uo pipefail

host="${1:?usage: probe-host.sh <host-name> <base-url> <probe.js> [width] [locale]}"
base="${2:?give a base URL}"
probe="${3:?give a probe file}"
width="${4:-720}"
locale="${5:-de}"
# SOME REFUSALS ARE TOASTS AND TOASTS EXPIRE. The shell's job refusal is gone from
# the page a few seconds after it appears, so a fixed four-second settle read a
# surface that HAD refused and no longer said so - which reads exactly like a
# fixture that never got there. `PROBE_HOST_SETTLE` shortens the wait for those.
settle="${PROBE_HOST_SETTLE:-4}"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
script="$here/hosts/$host.js"
[ -f "$script" ] || { echo "probe-host.sh: no $script" >&2; exit 2; }
[ -f "$probe" ] || { echo "probe-host.sh: no $probe" >&2; exit 2; }

want="$(sed -n 's|^// EXPECT: *||p' "$script" | head -1)"
[ -n "$want" ] || {
  echo "probe-host.sh: $host.js declares no '// EXPECT: <text>' line." >&2
  echo "  Add one naming what the state says on screen - the sentence, not a class." >&2
  exit 2
}

shot="$(mktemp /tmp/probe-host-XXXXXX.png)"
state="$(mktemp /tmp/probe-host-XXXXXX.js)"
trap 'rm -f "$shot" "$state"' EXIT

# The page's own words, not a selector: a refusal renders in a different element
# in nearly every app (a `[role=alert]`, an `.outcome`, a toast), and a selector
# list goes stale the way the gesture did. `innerText` is what a person reads.
cat > "$state" <<'JS'
return document.body.innerText.replace(/\s+/g, " ");
JS

# STDOUT ONLY. Merging stderr put a MESA driver warning on the last line one run
# in ten, so the state check compared the page against "MESA-EGL: warning: Ensure
# your X server supports DRI3" and refused a surface that was fine. A probe's
# answer is on stdout; the noise is not this check's business.
seen="$("$here/headless.sh" --url "$base/?locale=$locale" --out "$shot" --width "$width" \
  --settle "$settle" --host-script "$script" --probe-file "$state" 2>/dev/null | tail -1)"
case "$seen" in
  *"$want"*) ;;
  *)
    echo "probe-host.sh: $host did not reach its state; it says it should show:" >&2
    echo "  $want" >&2
    echo "  the page read: $(printf '%s' "$seen" | head -c 300)" >&2
    exit 2
    ;;
esac

"$here/headless.sh" --url "$base/?locale=$locale" --out "$shot" --width "$width" \
  --settle "$settle" --host-script "$script" --probe-file "$probe" 2>/dev/null | tail -1
