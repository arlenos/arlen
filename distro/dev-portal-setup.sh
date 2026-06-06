#!/bin/bash
# Lunaris portal — per-user dev-mode setup.
#
# Builds the daemon + picker binaries in debug mode, drops a
# symlinked D-Bus service file into the user's local services dir
# so D-Bus activation finds it without sudo, and prints the
# environment exports the dev-script needs in front of the daemon
# command.
#
# Idempotent: re-runs are safe.
#
# Usage:
#   ./distro/dev-portal-setup.sh
#
# Teardown:
#   ./distro/dev-portal-teardown.sh

set -euo pipefail

LUNARIS_PATH="${LUNARIS_PATH:-$HOME/Repositories/lunaris-sys}"
SRC="$LUNARIS_PATH/xdg-desktop-portal-lunaris"

USER_DBUS_SVC="$HOME/.local/share/dbus-1/services"
DBUS_SVC_NAME="org.freedesktop.impl.portal.desktop.lunaris.service"

DAEMON_BIN="$SRC/target/debug/xdg-desktop-portal-lunaris"
PICKER_BIN="$SRC/picker-ui/src-tauri/target/debug/xdg-desktop-portal-lunaris-picker"

echo "=== Lunaris portal dev setup ==="

# ── Build ──────────────────────────────────────────────────────

echo "[1/4] Building daemon (debug)..."
(cd "$SRC" && cargo build --bin xdg-desktop-portal-lunaris)

echo "[2/4] Building picker UI frontend..."
if [ ! -d "$SRC/picker-ui/node_modules" ]; then
    (cd "$SRC/picker-ui" && npm install)
fi
(cd "$SRC/picker-ui" && npm run build >/dev/null)

echo "[3/4] Building picker UI backend (debug)..."
(cd "$SRC/picker-ui/src-tauri" && cargo build)

# ── D-Bus service shim ─────────────────────────────────────────

# Generate a per-user .service file pointing at the debug binary.
# Using a real file (not a symlink to dist/) lets us substitute
# the dev path into the Exec line; the dist file targets the
# production install location.
echo "[4/4] Installing dev D-Bus service shim to $USER_DBUS_SVC"
mkdir -p "$USER_DBUS_SVC"
cat > "$USER_DBUS_SVC/$DBUS_SVC_NAME" <<EOF
[D-BUS Service]
Name=org.freedesktop.impl.portal.desktop.lunaris
Exec=$DAEMON_BIN
EOF

# ── Done ───────────────────────────────────────────────────────

echo
echo "=== Dev setup complete ==="
echo
echo "Verify (after start-dev.sh --with-portal restarts the frontend):"
echo "  busctl --user list | grep org.freedesktop.impl.portal.desktop.lunaris"
echo
echo "The dev start-dev.sh --with-portal flag handles env import +"
echo "frontend restart automatically. If you'd rather use the dev"
echo "service file outside the dev-script, run:"
echo "  systemctl --user import-environment XDG_CURRENT_DESKTOP XDG_DATA_DIRS"
echo "  systemctl --user restart xdg-desktop-portal"
echo "with XDG_CURRENT_DESKTOP=lunaris:wlroots and the augmented"
echo "XDG_DATA_DIRS exported in the calling shell."
