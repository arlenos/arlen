#!/bin/bash
# Arlen modulesd — per-user dev-mode setup.
#
# Builds the daemon in debug mode and drops systemd user unit files
# (pointing at the repo-local debug binary) into
# `~/.config/systemd/user/` so socket activation works without
# sudo.
#
# Idempotent: re-runs are safe.
#
# Usage:
#   ./distro/dev-modulesd-setup.sh
#
# Teardown:
#   systemctl --user disable --now arlen-modulesd.socket
#   rm ~/.config/systemd/user/arlen-modulesd.{service,socket}
#   systemctl --user daemon-reload

set -euo pipefail

ARLEN_PATH="${ARLEN_PATH:-$HOME/Repositories/arlenos}"
SRC="$ARLEN_PATH/modulesd"

USER_SYSTEMD="$HOME/.config/systemd/user"
DAEMON_BIN="$SRC/target/debug/arlen-modulesd"

echo "=== Arlen modulesd dev setup ==="

# ── Build ──────────────────────────────────────────────────────

echo "[1/3] Building daemon (debug)..."
(cd "$SRC" && cargo build --bin arlen-modulesd)

# ── User systemd units pointing at the debug binary ─────────────

echo "[2/3] Installing dev systemd units to $USER_SYSTEMD"
mkdir -p "$USER_SYSTEMD"

cat > "$USER_SYSTEMD/arlen-modulesd.service" <<EOF
[Unit]
Description=Arlen Module Runtime daemon (dev, debug binary)
Requires=arlen-modulesd.socket

[Service]
Type=notify
ExecStart=$DAEMON_BIN
Restart=on-failure
RestartSec=2

# Dev mode keeps the hardening off so it's easy to attach gdb /
# tweak file paths. install-modulesd.sh (production path) restores
# the full lockdown.

[Install]
WantedBy=default.target
EOF

# Reuse the production socket definition verbatim — it doesn't
# embed any production paths so the dev mode can share it.
install -m 0644 "$SRC/dist/arlen-modulesd.socket" \
    "$USER_SYSTEMD/arlen-modulesd.socket"

# ── Activate ───────────────────────────────────────────────────

echo "[3/3] Reloading + enabling user units"
systemctl --user daemon-reload
systemctl --user enable arlen-modulesd.socket

echo
echo "Dev setup done. To start the daemon:"
echo "  systemctl --user start arlen-modulesd.socket"
echo
echo "Status check:"
echo "  systemctl --user status arlen-modulesd"
echo
echo "Or skip socket activation entirely and run the binary directly:"
echo "  cd $SRC && cargo run --bin arlen-modulesd"
