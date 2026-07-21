#!/usr/bin/env bash
# Live harness cutover verification: render the harness under headless sway, drive
# a REAL prompt through it to the running pi engine (bridging the live drive socket
# into the sandbox XDG so drive.rs reaches it), wait for pi's streamed answer, and
# grim-capture pi's event stream landing on the A7 components. Needs the ai-engine
# daemon + pi + Ollama up (`just dev`) and the frontend served on :1423 (vite
# preview). Usage: shoot-harness-live.sh <out.png> [prompt] [wait-seconds]
set -uo pipefail
BIN="${BIN:-/home/tim/Repositories/arlen/target/debug/arlen-harness}"
OUT="${1:?usage: shoot-harness-live.sh <out.png> [prompt] [wait]}"
PROMPT="${2:-Summarize what I was working on recently.}"
WAIT="${3:-70}"
REALXDG="${REALXDG:-/run/user/1000}"

export XDG_RUNTIME_DIR="$(mktemp -d "${TMPDIR:-/tmp}/arlen-live-rt.XXXXXX")"
chmod 700 "$XDG_RUNTIME_DIR"
mkdir -p "$XDG_RUNTIME_DIR/arlen"
# Bridge the live engine drive socket into the sandbox XDG so the harness's
# drive.rs (which resolves $XDG_RUNTIME_DIR/arlen/ai-engine-drive.sock) reaches it.
ln -sf "$REALXDG/arlen/ai-engine-drive.sock" "$XDG_RUNTIME_DIR/arlen/ai-engine-drive.sock"
cleanup() { kill "${sway_pid:-0}" 2>/dev/null; rm -rf "$XDG_RUNTIME_DIR" 2>/dev/null; }
trap cleanup EXIT

cfg="$(mktemp)"
printf 'output HEADLESS-1 resolution 1280x800\nexec env GDK_BACKEND=wayland %q >/tmp/arlen-live-app.log 2>&1\n' "$BIN" > "$cfg"
WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 sway -c "$cfg" >/tmp/arlen-live-sway.log 2>&1 &
sway_pid=$!
sleep 24
WD="$(ls "$XDG_RUNTIME_DIR" 2>/dev/null | grep -E '^wayland-[0-9]+$' | head -1)"
if [ -z "$WD" ]; then echo "no headless sway socket - refusing to grab" >&2; exit 1; fi

# Focus the composer + type the prompt + submit. The composer input auto-focuses;
# a leading Tab/click is unnecessary for the chat input, but type slowly so the
# webview keystroke handler keeps up.
sleep 3
# On a CLEARED (empty) session the harness shows suggestion chips at fixed
# positions and the composer is not auto-focused, so a fixed-position ydotool
# click on a chip is the most reliable trigger (no typing/focus dependency): the
# chip sends its prompt straight to pi. ydotoold binds the REAL runtime dir, so
# point ydotool at it (the shoot's XDG is a temp dir for sway isolation).
export YDOTOOL_SOCKET="${YDOTOOL_SOCKET:-$REALXDG/.ydotool_socket}"
# CHIP_XY overrides the chip position; default = the first suggestion chip
# ("What did I work on yesterday?") at ~766,402 in the 1280x800 render.
CX="${CHIP_X:-766}"; CY="${CHIP_Y:-402}"
sleep 1
ydotool mousemove -a "$CX" "$CY" >/tmp/arlen-live-type.log 2>&1
sleep 1
ydotool click 0xC0 >>/tmp/arlen-live-type.log 2>&1
# Wait for pi -> Ollama/qwen to stream the answer onto the A7 components.
sleep "$WAIT"
WAYLAND_DISPLAY="$WD" grim "$OUT"; rc=$?
echo "live shot rc=$rc -> $OUT"
exit $rc
