#!/bin/bash
# Lunaris OS Development Script
# Kills all running instances and launches everything in tmux
#
# Usage: ./dev.sh
# Stop:  tmux kill-session -t lunaris
#
# Tmux controls:
#   Ctrl+B, n           - Next window
#   Ctrl+B, p           - Previous window
#   Ctrl+B, 0-4         - Jump to window by number
#   Ctrl+B, d           - Detach (processes keep running)
#   tmux attach -t lunaris - Reattach

set -e

echo "=== Lunaris Dev Environment ==="

# Base path
LUNARIS_PATH="${LUNARIS_PATH:-$HOME/Repositories/lunaris-sys}"

# Log directory. Every component tees its stdout+stderr to a fresh file
# so the user can grep without opening DevTools / scrolling tmux history.
LOG_DIR="$LUNARIS_PATH/logs"
mkdir -p "$LOG_DIR"
# Wipe previous session logs so `tail -f` / `grep` only shows this run.
rm -f "$LOG_DIR"/*.log

# Event-Bus socket dir. Previously used /run/lunaris/ which required
# root to create; tmux windows inherit no sudo ticket so the daemon
# silently timed out at the password prompt and every consumer spent
# the whole session reconnect-looping. Switched to the per-user
# XDG_RUNTIME_DIR so the whole stack runs as the login user.
EVENT_BUS_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/lunaris"
mkdir -p "$EVENT_BUS_DIR"
export LUNARIS_PRODUCER_SOCKET="$EVENT_BUS_DIR/event-bus-producer.sock"
export LUNARIS_CONSUMER_SOCKET="$EVENT_BUS_DIR/event-bus-consumer.sock"

# Default log filter. Silences `zbus` INFO-level handshake/dispatch
# chatter (~300 lines per shell session — see notes in logs/).
# Override from the calling shell if deeper tracing is needed.
RUST_LOG_FILTER="${RUST_LOG:-info,zbus=warn,tracing=warn}"

echo "[1/4] Killing existing processes..."

# Kill Lunaris processes
pkill -9 -f cosmic-comp 2>/dev/null || true
pkill -9 -f "event-bus" 2>/dev/null || true
pkill -9 -f "target/debug/knowledge" 2>/dev/null || true
pkill -9 -f lunaris-notifyd 2>/dev/null || true
pkill -9 -f "desktop-shell" 2>/dev/null || true

# Kill other notification daemons that might hold D-Bus name
pkill -9 -f dunst 2>/dev/null || true
pkill -9 -f mako 2>/dev/null || true
pkill -9 -f swaync 2>/dev/null || true
pkill -9 -f xfce4-notifyd 2>/dev/null || true

echo "[2/4] Cleaning up sockets..."

# Stale user sockets from previous run
rm -f "$EVENT_BUS_DIR"/event-bus-*.sock 2>/dev/null || true
# Legacy /run/lunaris/ leftovers — only touch if we previously created
# them. Silent-skip if the dir doesn't exist so we don't need sudo just
# for the cleanup pass.
if [ -d /run/lunaris ]; then
    sudo rm -f /run/lunaris/event-bus-*.sock 2>/dev/null || true
fi

sleep 1

echo "[3/4] Killing existing tmux session..."

tmux kill-session -t lunaris 2>/dev/null || true

echo "[4/4] Starting tmux session..."

# Each component runs with RUST_LOG=info and tees combined stdout+stderr
# into a dedicated file under $LOG_DIR, so `tail -f ~/Repositories/lunaris-sys/logs/compositor.log`
# works regardless of whether the user is attached to tmux.
# All components inherit LUNARIS_PRODUCER_SOCKET / LUNARIS_CONSUMER_SOCKET
# from this shell's exported environment so they agree on the socket
# path without a hardcoded constant.

# Every `send-keys` explicitly prefixes the three env vars so an
# interactive shell's rc-file (bashrc, zshrc) cannot silently strip
# them. Quote once with single-quotes so the shell inside tmux expands
# `$RUST_LOG_FILTER` from the final command line, not from the parent.
ENV_PREFIX="RUST_LOG='$RUST_LOG_FILTER' LUNARIS_PRODUCER_SOCKET='$LUNARIS_PRODUCER_SOCKET' LUNARIS_CONSUMER_SOCKET='$LUNARIS_CONSUMER_SOCKET'"

# Window 0: Compositor
tmux new-session -d -s lunaris -n compositor
tmux send-keys -t lunaris:compositor "cd $LUNARIS_PATH/compositor && $ENV_PREFIX cargo run --bin cosmic-comp 2>&1 | tee $LOG_DIR/compositor.log" Enter

# Window 1: Event Bus (user-owned socket in XDG_RUNTIME_DIR — no sudo)
tmux new-window -t lunaris -n eventbus
tmux send-keys -t lunaris:eventbus "sleep 2 && cd $LUNARIS_PATH/event-bus && $ENV_PREFIX cargo run 2>&1 | tee $LOG_DIR/event-bus.log" Enter

# Window 2: Knowledge
tmux new-window -t lunaris -n knowledge
tmux send-keys -t lunaris:knowledge "sleep 6 && cd $LUNARIS_PATH/knowledge && $ENV_PREFIX cargo run 2>&1 | tee $LOG_DIR/knowledge.log" Enter

# Window 3: Notification Daemon
tmux new-window -t lunaris -n notifyd
tmux send-keys -t lunaris:notifyd "sleep 7 && cd $LUNARIS_PATH/notification-daemon && $ENV_PREFIX cargo run 2>&1 | tee $LOG_DIR/notifyd.log" Enter

# Window 4: Desktop Shell (Tauri). `cargo tauri dev` also spawns the
# Vite dev server; tee captures both.
tmux new-window -t lunaris -n shell
tmux send-keys -t lunaris:shell "sleep 10 && cd $LUNARIS_PATH/desktop-shell && $ENV_PREFIX WAYLAND_DISPLAY=wayland-2 cargo tauri dev 2>&1 | tee $LOG_DIR/desktop-shell.log" Enter

# Select shell window
tmux select-window -t lunaris:shell

echo "=== Lunaris Dev Environment Started ==="
echo ""
echo "Attaching to tmux session..."
echo "  Ctrl+B, n       - Next window"
echo "  Ctrl+B, p       - Previous window"
echo "  Ctrl+B, 0-4     - Jump to window"
echo "  Ctrl+B, d       - Detach"
echo ""
echo "Windows: 0:compositor  1:eventbus  2:knowledge  3:notifyd  4:shell"
echo ""
echo "Logs: tail -f $LOG_DIR/compositor.log"
echo "      grep -E '...' $LOG_DIR/compositor.log"
echo ""

tmux attach -t lunaris
