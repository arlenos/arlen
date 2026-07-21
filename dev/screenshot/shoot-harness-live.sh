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
# Submit with wtype (a wayland VIRTUAL KEYBOARD - works under WLR_LIBINPUT_NO_DEVICES=1,
# unlike ydotool/uinput which sway then ignores). Focus the composer first with a
# couple of Shift+Tabs from the app's initial focus (the composer is the last
# focusable), type, settle, then Enter (repeated - the growing-textarea handler can
# swallow the first). Needs a POPULATED conversation so the composer is present.
# The composer auto-focuses on load, so type straight in (no Tab dance - Shift+Tab
# mangled the text + the stray '@' opened the mention picker). Type slowly, settle,
# then Enter (twice - the growing-textarea handler can swallow the first).
sleep 1
WAYLAND_DISPLAY="$WD" wtype -s 45 "$PROMPT" >/tmp/arlen-live-type.log 2>&1
sleep 3
WAYLAND_DISPLAY="$WD" wtype -k Return >>/tmp/arlen-live-type.log 2>&1
sleep 1
WAYLAND_DISPLAY="$WD" wtype -k Return >>/tmp/arlen-live-type.log 2>&1
# Wait for pi -> Ollama/qwen to stream the answer onto the A7 components.
sleep "$WAIT"
WAYLAND_DISPLAY="$WD" grim "$OUT"; rc=$?
echo "live shot rc=$rc -> $OUT"
exit $rc
