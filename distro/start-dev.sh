#!/bin/bash
# Lunaris OS Development Script
# Kills all running instances and launches everything in tmux
#
# Usage:
#   ./start-dev.sh                  # 5-window default stack
#   ./start-dev.sh --with-portal    # plus app-settings + xdg-portal-lunaris
#
# Stop:  tmux kill-session -t lunaris
#
# Tmux controls:
#   Ctrl+B, n           - Next window
#   Ctrl+B, p           - Previous window
#   Ctrl+B, 0-N         - Jump to window by number
#   Ctrl+B, d           - Detach (processes keep running)
#   tmux attach -t lunaris - Reattach

set -e

echo "=== Lunaris Dev Environment ==="

# Parse flags. `--with-portal` adds two windows for the
# Pre-Phase-6 Sprint F portal stack (app-settings + portal-daemon)
# and exports XDG_CURRENT_DESKTOP / XDG_DATA_DIRS so the frontend
# portal daemon dispatches to our backend. Default off so existing
# dev workflows keep their familiar 5-window layout.
WITH_PORTAL=0
for arg in "$@"; do
    case "$arg" in
        --with-portal) WITH_PORTAL=1 ;;
        *) echo "Unknown flag: $arg" >&2; exit 1 ;;
    esac
done

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
# Knowledge-daemon socket: also in XDG so no sudo needed. Without
# this, the daemon falls back to /run/lunaris/knowledge.sock where
# the bind() fails with EACCES and the daemon terminates.
export LUNARIS_DAEMON_SOCKET="$EVENT_BUS_DIR/knowledge.sock"

TIMELINE_MOUNT="$HOME/.timeline"

# Default log filter. Silences `zbus` INFO-level handshake/dispatch
# chatter (~300 lines per shell session — see notes in logs/).
# Override from the calling shell if deeper tracing is needed.
RUST_LOG_FILTER="${RUST_LOG:-info,zbus=warn,tracing=warn}"

# Step counter and total: total grows by 1 (extra build step) when
# --with-portal is set so the user can see overall progress.
STEP_TOTAL=6
if [ "$WITH_PORTAL" -eq 1 ]; then
    STEP_TOTAL=7
fi

echo "[1/$STEP_TOTAL] Killing existing processes..."

# Kill Lunaris processes
pkill -9 -f cosmic-comp 2>/dev/null || true
pkill -9 -f "event-bus" 2>/dev/null || true
pkill -9 -f "target/debug/knowledge" 2>/dev/null || true
pkill -9 -f lunaris-notifyd 2>/dev/null || true
pkill -9 -f "desktop-shell" 2>/dev/null || true
if [ "$WITH_PORTAL" -eq 1 ]; then
    pkill -9 -f "target/debug/lunaris-settings" 2>/dev/null || true
    pkill -9 -f "target/debug/xdg-desktop-portal-lunaris" 2>/dev/null || true
    pkill -9 -f xdg-desktop-portal-lunaris-picker 2>/dev/null || true
fi

# Kill other notification daemons that might hold D-Bus name
pkill -9 -f dunst 2>/dev/null || true
pkill -9 -f mako 2>/dev/null || true
pkill -9 -f swaync 2>/dev/null || true
pkill -9 -f xfce4-notifyd 2>/dev/null || true

echo "[2/$STEP_TOTAL] Cleaning up stale FUSE mount at $TIMELINE_MOUNT..."

# The Knowledge daemon FUSE-mounts ~/.timeline. A hard-kill (pkill -9
# above) doesn't trigger the FUSE exit handler — the kernel keeps the
# mount registered but the userspace process is gone, so any access
# returns ENOTCONN ("Transport endpoint is not connected"). The next
# daemon start then fails with `File exists (os error 17)` when it
# tries to re-mount, and the zombie mount stays. Cleanup: three
# fallback levels, fail fast if none work.
if mountpoint -q "$TIMELINE_MOUNT" 2>/dev/null \
   || findmnt -M "$TIMELINE_MOUNT" >/dev/null 2>&1; then
    echo "  found mount — attempting unmount"
    if fusermount -u "$TIMELINE_MOUNT" 2>/dev/null; then
        echo "  unmounted cleanly"
    elif fusermount -uz "$TIMELINE_MOUNT" 2>/dev/null; then
        echo "  unmounted lazily (will finalise when last ref released)"
    elif sudo umount -l "$TIMELINE_MOUNT" 2>/dev/null; then
        echo "  unmounted via sudo umount -l"
    else
        echo "ERROR: could not unmount $TIMELINE_MOUNT" >&2
        echo "  Dev session cannot start with a stale FUSE mount." >&2
        echo "  Fix manually:  sudo umount -l $TIMELINE_MOUNT" >&2
        exit 1
    fi
else
    echo "  no stale mount"
fi

echo "[3/$STEP_TOTAL] Cleaning up sockets..."

# Stale user sockets from previous run
rm -f "$EVENT_BUS_DIR"/event-bus-*.sock \
      "$EVENT_BUS_DIR"/knowledge.sock \
      "$EVENT_BUS_DIR"/portal-picker.sock \
      2>/dev/null || true
# Legacy /run/lunaris/ leftovers — only touch if we previously created
# them. Silent-skip if the dir doesn't exist so we don't need sudo just
# for the cleanup pass.
if [ -d /run/lunaris ]; then
    sudo rm -f /run/lunaris/event-bus-*.sock /run/lunaris/knowledge.sock 2>/dev/null || true
fi

sleep 1

echo "[4/$STEP_TOTAL] Killing existing tmux session..."

tmux kill-session -t lunaris 2>/dev/null || true

# Pre-build the picker-ui binary up front so the portal daemon's
# spawn does not pay a 30 s+ first-pick cargo-compile penalty.
# Skipped without --with-portal so the default workflow stays
# fast.
if [ "$WITH_PORTAL" -eq 1 ]; then
    PORTAL_SRC="$LUNARIS_PATH/xdg-desktop-portal-lunaris"
    PICKER_BIN="$PORTAL_SRC/picker-ui/src-tauri/target/debug/xdg-desktop-portal-lunaris-picker"
    echo "[5/$STEP_TOTAL] Pre-building portal picker-ui (warm-cache: ~2s, cold: ~30s)..."
    if [ ! -d "$PORTAL_SRC/picker-ui/node_modules" ]; then
        (cd "$PORTAL_SRC/picker-ui" && npm install >/dev/null 2>&1)
    fi
    (cd "$PORTAL_SRC/picker-ui" && npm run build >/dev/null 2>&1)
    (cd "$PORTAL_SRC/picker-ui/src-tauri" && cargo build 2>&1) | tail -5
    if [ ! -x "$PICKER_BIN" ]; then
        echo "ERROR: picker binary not built at $PICKER_BIN" >&2
        exit 1
    fi
    SESSION_STEP=6
    SHELL_STEP=7
else
    SESSION_STEP=5
    SHELL_STEP=6
fi

echo "[$SESSION_STEP/$STEP_TOTAL] Starting tmux session..."

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
ENV_PREFIX="RUST_LOG='$RUST_LOG_FILTER' LUNARIS_PRODUCER_SOCKET='$LUNARIS_PRODUCER_SOCKET' LUNARIS_CONSUMER_SOCKET='$LUNARIS_CONSUMER_SOCKET' LUNARIS_DAEMON_SOCKET='$LUNARIS_DAEMON_SOCKET'"

# Portal-mode env additions. XDG_CURRENT_DESKTOP makes the
# frontend portal daemon match our `UseIn=lunaris;` config (kept
# additive so wlroots-aware fallbacks still work). XDG_DATA_DIRS
# is APPENDED — prepending would let dev portal configs shadow
# system XDG specs and break unrelated apps.
if [ "$WITH_PORTAL" -eq 1 ]; then
    PORTAL_DIST="$LUNARIS_PATH/xdg-desktop-portal-lunaris/dist"
    EXISTING_DATA_DIRS="${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
    PORTAL_DATA_DIRS="$EXISTING_DATA_DIRS:$PORTAL_DIST"
    EXISTING_DESKTOP="${XDG_CURRENT_DESKTOP:-wlroots}"
    case ":$EXISTING_DESKTOP:" in
        *:lunaris:*) PORTAL_DESKTOP="$EXISTING_DESKTOP" ;;
        *) PORTAL_DESKTOP="lunaris:$EXISTING_DESKTOP" ;;
    esac
    # Idle timeout extended to 30 min for dev sessions so the daemon
    # doesn't exit mid-debug while you're staring at logs. Production
    # leaves it at the 60 s default.
    PORTAL_ENV_PREFIX="$ENV_PREFIX XDG_CURRENT_DESKTOP='$PORTAL_DESKTOP' XDG_DATA_DIRS='$PORTAL_DATA_DIRS' LUNARIS_PORTAL_PICKER_BIN='$PICKER_BIN' LUNARIS_PORTAL_IDLE_TIMEOUT_SECS='1800'"

    # Codex review: tmux child env doesn't reach the systemd
    # user-manager-spawned `xdg-desktop-portal.service`, which is
    # the process that actually selects the backend. Without these
    # two steps the frontend keeps using whatever backend was
    # selected at session start (typically GTK) and the dev script
    # silently tests the wrong implementation.
    export XDG_CURRENT_DESKTOP="$PORTAL_DESKTOP"
    export XDG_DATA_DIRS="$PORTAL_DATA_DIRS"
    if command -v systemctl >/dev/null 2>&1; then
        echo "    importing portal env into systemd user manager"
        systemctl --user import-environment XDG_CURRENT_DESKTOP XDG_DATA_DIRS \
            >/dev/null 2>&1 || \
            echo "    WARNING: import-environment failed; frontend may not see lunaris env" >&2
        echo "    restarting xdg-desktop-portal so it picks up the new env"
        systemctl --user restart xdg-desktop-portal >/dev/null 2>&1 || \
            echo "    WARNING: restart xdg-desktop-portal failed; check systemctl --user status xdg-desktop-portal" >&2
    else
        echo "    WARNING: systemctl not found; frontend portal selector likely unchanged" >&2
    fi
fi

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

if [ "$WITH_PORTAL" -eq 1 ]; then
    # Window 5: app-settings (Tauri). The Settings DirectoryPicker
    # is the unconfined-side E2E test for the portal plugin.
    tmux new-window -t lunaris -n settings
    tmux send-keys -t lunaris:settings "sleep 12 && cd $LUNARIS_PATH/app-settings && $PORTAL_ENV_PREFIX WAYLAND_DISPLAY=wayland-2 cargo tauri dev 2>&1 | tee $LOG_DIR/app-settings.log" Enter

    # Window 6: portal daemon. Spawned AFTER the picker binary
    # build (pre-build step) so the daemon's pre-warm finds it.
    # WAYLAND_DISPLAY=wayland-2 is critical: the daemon spawns the
    # picker-ui as a subprocess, and the subprocess inherits the
    # daemon's env. Without this, picker-ui connects to the host
    # session (whichever Wayland the dev shell is on) instead of
    # the nested cosmic-comp the rest of the stack runs against.
    tmux new-window -t lunaris -n portal-daemon
    tmux send-keys -t lunaris:portal-daemon "sleep 8 && cd $LUNARIS_PATH/xdg-desktop-portal-lunaris && $PORTAL_ENV_PREFIX WAYLAND_DISPLAY=wayland-2 cargo run --bin xdg-desktop-portal-lunaris 2>&1 | tee $LOG_DIR/portal-daemon.log" Enter
fi

# Select shell window
tmux select-window -t lunaris:shell

echo "[$SHELL_STEP/$STEP_TOTAL] Waiting for daemons to come up..."

# Give every component long enough to build (first run) and bind its
# socket. The Shell (Tauri) is the slowest — we don't wait for it, the
# health check below only covers the backend processes.
sleep 15

MISSING=()
pgrep -f cosmic-comp            >/dev/null || MISSING+=(compositor)
pgrep -f "event-bus/target"     >/dev/null \
    || pgrep -f "target/debug/event-bus" >/dev/null \
    || MISSING+=(event-bus)
pgrep -f "target/debug/knowledge" >/dev/null || MISSING+=(knowledge)
pgrep -f lunaris-notifyd        >/dev/null || MISSING+=(notification-daemon)
if [ "$WITH_PORTAL" -eq 1 ]; then
    pgrep -f "target/debug/xdg-desktop-portal-lunaris$" >/dev/null \
        || MISSING+=(portal-daemon)
fi

if [ ${#MISSING[@]} -eq 0 ]; then
    echo "  all backend daemons are running"
else
    echo "WARNING: the following daemons are NOT running: ${MISSING[*]}" >&2
    echo "  Check logs: $LOG_DIR/<name>.log" >&2
fi

# Knowledge socket is the single most common breakage: if it isn't
# bound, the shell's 'Projects' / 'Recent Files' silently degrade to
# empty. Surface that here instead of letting it be a mystery later.
if [ ! -S "$LUNARIS_DAEMON_SOCKET" ]; then
    echo "WARNING: knowledge socket not found at $LUNARIS_DAEMON_SOCKET" >&2
    echo "  tail -f $LOG_DIR/knowledge.log to diagnose" >&2
fi

# Portal-side health check: verify D-Bus name registration and the
# IPC socket the picker UI binds. Race-tolerant: we already
# slept 15s, plus retry busctl twice in case name-bind landed
# late.
if [ "$WITH_PORTAL" -eq 1 ]; then
    if [ ! -S "$EVENT_BUS_DIR/portal-picker.sock" ]; then
        echo "WARNING: portal picker IPC socket not found at $EVENT_BUS_DIR/portal-picker.sock" >&2
    fi
    PORTAL_BUS_OK=0
    for _ in 1 2 3; do
        if busctl --user list 2>/dev/null \
            | grep -q org.freedesktop.impl.portal.desktop.lunaris; then
            PORTAL_BUS_OK=1
            break
        fi
        sleep 1
    done
    if [ "$PORTAL_BUS_OK" -eq 0 ]; then
        echo "WARNING: portal D-Bus name not registered" >&2
        echo "  tail -f $LOG_DIR/portal-daemon.log to diagnose" >&2
    fi
fi

echo "=== Lunaris Dev Environment Started ==="
echo ""
echo "Attaching to tmux session..."
echo "  Ctrl+B, n       - Next window"
echo "  Ctrl+B, p       - Previous window"
echo "  Ctrl+B, 0-N     - Jump to window"
echo "  Ctrl+B, d       - Detach"
echo ""
if [ "$WITH_PORTAL" -eq 1 ]; then
    echo "Windows: 0:compositor  1:eventbus  2:knowledge  3:notifyd  4:shell  5:settings  6:portal-daemon"
else
    echo "Windows: 0:compositor  1:eventbus  2:knowledge  3:notifyd  4:shell"
fi
echo ""
echo "Logs: tail -f $LOG_DIR/compositor.log"
echo "      grep -E '...' $LOG_DIR/compositor.log"
echo ""

tmux attach -t lunaris
