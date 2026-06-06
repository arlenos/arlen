#!/usr/bin/env bash
# Start the Lunaris Settings app inside the running nested cosmic-comp
# session and pipe stdout/stderr to ~/Repositories/lunaris-sys/logs/settings.log.
#
# Prereqs:
#  - `start-dev.sh` is running in tmux, so wayland-2 is up.
#  - Compositor + desktop-shell are already alive.
#
# Usage:
#   ./distro/start-settings.sh
#
# `cargo tauri dev` blocks; Ctrl+C to stop. The Vite dev server and the
# Rust binary both share the log file via `tee`.

set -e

LUNARIS_PATH="$HOME/Repositories/lunaris-sys"
LOG_DIR="$LUNARIS_PATH/logs"
mkdir -p "$LOG_DIR"

# Match the compositor / desktop-shell wayland socket and verbosity.
# `RUST_LOG=info` keeps the night-light + display traces visible.
export WAYLAND_DISPLAY=wayland-2
export RUST_LOG=info

cd "$LUNARIS_PATH/app-settings"
cargo tauri dev 2>&1 | tee "$LOG_DIR/settings.log"
