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

echo "[1/4] Killing existing processes..."

# Kill Lunaris processes
pkill -9 -f cosmic-comp 2>/dev/null || true
sudo pkill -9 -f "event-bus" 2>/dev/null || true
pkill -9 -f "target/debug/knowledge" 2>/dev/null || true
pkill -9 -f lunaris-notifyd 2>/dev/null || true
pkill -9 -f "desktop-shell" 2>/dev/null || true

# Kill other notification daemons that might hold D-Bus name
pkill -9 -f dunst 2>/dev/null || true
pkill -9 -f mako 2>/dev/null || true
pkill -9 -f swaync 2>/dev/null || true
pkill -9 -f xfce4-notifyd 2>/dev/null || true

echo "[2/4] Cleaning up sockets..."

sudo rm -f /run/lunaris/*.sock 2>/dev/null || true
sudo mkdir -p /run/lunaris
sudo chmod 777 /run/lunaris

sleep 1

echo "[3/4] Killing existing tmux session..."

tmux kill-session -t lunaris 2>/dev/null || true

echo "[4/4] Starting tmux session..."

# Window 0: Compositor
tmux new-session -d -s lunaris -n compositor
tmux send-keys -t lunaris:compositor "cd $LUNARIS_PATH/compositor && RUST_LOG=warn cargo run --bin cosmic-comp" Enter

# Window 1: Event Bus (runs as root for /run/lunaris/ sockets)
tmux new-window -t lunaris -n eventbus
tmux send-keys -t lunaris:eventbus "sleep 2 && cd $LUNARIS_PATH/event-bus && sudo cargo run" Enter

# Fix event bus socket permissions after it has time to create them.
# The directory is 777 but the sockets are created by root (event-bus).
# This runs in the script's own shell (where sudo already has a ticket
# from the cleanup step above), not inside tmux.
(sleep 4 && sudo chmod 666 /run/lunaris/*.sock 2>/dev/null || true) &

# Window 2: Knowledge (no sudo needed — /run/lunaris/ is already 777)
tmux new-window -t lunaris -n knowledge
tmux send-keys -t lunaris:knowledge "sleep 6 && cd $LUNARIS_PATH/knowledge && cargo run" Enter

# Window 3: Notification Daemon
tmux new-window -t lunaris -n notifyd
tmux send-keys -t lunaris:notifyd "sleep 7 && cd $LUNARIS_PATH/notification-daemon && cargo run" Enter

# Window 4: Desktop Shell
tmux new-window -t lunaris -n shell
tmux send-keys -t lunaris:shell "sleep 10 && cd $LUNARIS_PATH/desktop-shell && WAYLAND_DISPLAY=wayland-2 cargo tauri dev" Enter

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

tmux attach -t lunaris
