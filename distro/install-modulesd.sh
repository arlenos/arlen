#!/bin/bash
# Lunaris Module Runtime daemon — system-wide install.
#
# Copies the built modulesd binary plus systemd unit files into the
# standard user-service locations so socket activation can kick the
# daemon awake on first shell connection.
#
# Usage:
#   cd ~/Repositories/lunaris-sys
#   ./distro/install-modulesd.sh
#
# For dev work (no sudo, repo-local debug binary) use
# ./distro/dev-modulesd-setup.sh instead.

set -euo pipefail

# ── Configuration ──────────────────────────────────────────────

LUNARIS_PATH="${LUNARIS_PATH:-$HOME/Repositories/lunaris-sys}"
SRC="$LUNARIS_PATH/modulesd"

# Source artefacts. Built via:
#   (cd "$SRC" && cargo build --release)
DAEMON_BIN="$SRC/target/release/lunaris-modulesd"
SYSTEMD_SERVICE="$SRC/dist/lunaris-modulesd.service"
SYSTEMD_SOCKET="$SRC/dist/lunaris-modulesd.socket"

# Destinations. modulesd is a *user* service (per-session daemon)
# so its unit files live under `/usr/lib/systemd/user/`.
DEST_LIBEXEC="/usr/lib/lunaris/libexec"
DEST_SYSTEMD_USER="/usr/lib/systemd/user"

# ── Pre-flight ─────────────────────────────────────────────────

if [ "$(id -u)" -ne 0 ]; then
    echo "Re-executing under sudo for /usr writes..."
    exec sudo --preserve-env=LUNARIS_PATH "$0" "$@"
fi

echo "=== Lunaris modulesd install ==="

if [ ! -x "$DAEMON_BIN" ]; then
    echo "ERROR: daemon binary not found at $DAEMON_BIN" >&2
    echo "  Build with: (cd $SRC && cargo build --release)" >&2
    exit 1
fi
for src in "$SYSTEMD_SERVICE" "$SYSTEMD_SOCKET"; do
    if [ ! -f "$src" ]; then
        echo "ERROR: source file missing: $src" >&2
        exit 1
    fi
done

# Stop any running instance before overwriting the binary so the
# new one is picked up on next activation. Best effort — if the
# unit was never installed, this is a no-op.
if systemctl --user is-active --quiet lunaris-modulesd.service 2>/dev/null; then
    echo "Stopping running modulesd.service before upgrade..."
    systemctl --user stop lunaris-modulesd.service || true
fi

# ── Backup-if-diff helper ──────────────────────────────────────

backup_if_diff() {
    local src="$1"
    local dest="$2"
    if [ -f "$dest" ] && ! cmp -s "$src" "$dest"; then
        local stamp
        stamp=$(date +%Y%m%d-%H%M%S)
        cp -a "$dest" "$dest.bak.$stamp"
        echo "  backed up old $dest -> $dest.bak.$stamp"
    fi
}

# ── Install ────────────────────────────────────────────────────

echo "[1/3] Installing daemon binary to $DEST_LIBEXEC"
install -d "$DEST_LIBEXEC"
backup_if_diff "$DAEMON_BIN" "$DEST_LIBEXEC/lunaris-modulesd"
install -m 0755 "$DAEMON_BIN" "$DEST_LIBEXEC/lunaris-modulesd"

echo "[2/3] Installing systemd user units to $DEST_SYSTEMD_USER"
install -d "$DEST_SYSTEMD_USER"
backup_if_diff "$SYSTEMD_SERVICE" "$DEST_SYSTEMD_USER/lunaris-modulesd.service"
install -m 0644 "$SYSTEMD_SERVICE" "$DEST_SYSTEMD_USER/lunaris-modulesd.service"
backup_if_diff "$SYSTEMD_SOCKET" "$DEST_SYSTEMD_USER/lunaris-modulesd.socket"
install -m 0644 "$SYSTEMD_SOCKET" "$DEST_SYSTEMD_USER/lunaris-modulesd.socket"

echo "[3/3] Reloading systemd user daemon configuration"
# Per-user daemon reload runs as the invoking user, not root.
# `loginctl enable-linger` is the user's call, not ours; if they
# want the daemon available outside a login session they enable
# linger themselves.
if [ -n "${SUDO_USER:-}" ]; then
    sudo -u "$SUDO_USER" systemctl --user daemon-reload || true
fi

echo
echo "Install done. To activate:"
echo "  systemctl --user enable --now lunaris-modulesd.socket"
echo
echo "Status check:"
echo "  systemctl --user status lunaris-modulesd"
