#!/bin/bash
# Lunaris xdg-desktop-portal backend — system-wide install.
#
# Copies the built daemon + picker binaries plus service files
# into the standard freedesktop locations so the frontend portal
# daemon dispatches FileChooser/OpenURI calls to Lunaris.
#
# Usage:
#   cd ~/Repositories/lunaris-sys
#   ./distro/install-portal.sh
#
# This is the production path; for dev work (no sudo, repo-local
# binaries) use ./distro/dev-portal-setup.sh instead.

set -euo pipefail

# ── Configuration ──────────────────────────────────────────────

LUNARIS_PATH="${LUNARIS_PATH:-$HOME/Repositories/lunaris-sys}"
SRC="$LUNARIS_PATH/xdg-desktop-portal-lunaris"

# Source artefacts. Built via:
#   (cd "$SRC" && cargo build --release)
#   (cd "$SRC/picker-ui" && npm run build)
#   (cd "$SRC/picker-ui/src-tauri" && cargo build --release)
DAEMON_BIN="$SRC/target/release/xdg-desktop-portal-lunaris"
PICKER_BIN="$SRC/picker-ui/src-tauri/target/release/xdg-desktop-portal-lunaris-picker"
DBUS_SERVICE="$SRC/dist/dbus/org.freedesktop.impl.portal.desktop.lunaris.service"
SYSTEMD_UNIT="$SRC/dist/systemd/xdg-desktop-portal-lunaris.service"
PORTAL_CONFIG="$SRC/dist/xdg-desktop-portal/portals/lunaris.portal"

# Destinations.
DEST_LIBEXEC="/usr/lib/lunaris/libexec"
DEST_DBUS_SVC="/usr/share/dbus-1/services"
DEST_SYSTEMD_UNIT="/usr/lib/systemd/user"
DEST_PORTAL_CFG="/usr/share/xdg-desktop-portal/portals"
DEST_ENV_GEN="/usr/lib/systemd/user-environment-generators"
ENV_GEN_NAME="30-lunaris"

# ── Pre-flight ─────────────────────────────────────────────────

# We need root for /usr/* writes. Prefer to elevate the script
# itself once rather than ask the user to type sudo for every cp.
if [ "$(id -u)" -ne 0 ]; then
    echo "Re-executing under sudo for /usr writes..."
    exec sudo --preserve-env=LUNARIS_PATH "$0" "$@"
fi

echo "=== Lunaris portal install ==="

if [ ! -x "$DAEMON_BIN" ]; then
    echo "ERROR: daemon binary not found at $DAEMON_BIN" >&2
    echo "  Build with: (cd $SRC && cargo build --release)" >&2
    exit 1
fi
if [ ! -x "$PICKER_BIN" ]; then
    echo "ERROR: picker binary not found at $PICKER_BIN" >&2
    echo "  Build with: (cd $SRC/picker-ui && npm install && npm run build) && \\" >&2
    echo "             (cd $SRC/picker-ui/src-tauri && cargo build --release)" >&2
    exit 1
fi
for src in "$DBUS_SERVICE" "$SYSTEMD_UNIT" "$PORTAL_CONFIG"; do
    if [ ! -f "$src" ]; then
        echo "ERROR: source file missing: $src" >&2
        exit 1
    fi
done

# ── Backup-if-diff helper ──────────────────────────────────────

# Only back up an existing target if its content differs from what
# we are about to install. Re-runs of the installer with no source
# changes therefore leave no .bak trail.
backup_if_diff() {
    local src="$1"
    local dest="$2"
    if [ -f "$dest" ] && ! cmp -s "$src" "$dest"; then
        local stamp
        stamp=$(date +%Y%m%d-%H%M%S)
        cp -a "$dest" "$dest.bak.$stamp"
        echo "  backed up modified $dest -> $dest.bak.$stamp"
    fi
}

# ── Install ────────────────────────────────────────────────────

echo "[1/5] Installing binaries to $DEST_LIBEXEC"
mkdir -p "$DEST_LIBEXEC"
install -m 0755 "$DAEMON_BIN" "$DEST_LIBEXEC/xdg-desktop-portal-lunaris"
install -m 0755 "$PICKER_BIN" "$DEST_LIBEXEC/xdg-desktop-portal-lunaris-picker"

echo "[2/5] Installing D-Bus service file to $DEST_DBUS_SVC"
mkdir -p "$DEST_DBUS_SVC"
backup_if_diff "$DBUS_SERVICE" "$DEST_DBUS_SVC/org.freedesktop.impl.portal.desktop.lunaris.service"
install -m 0644 "$DBUS_SERVICE" "$DEST_DBUS_SVC/"

echo "[3/5] Installing systemd unit to $DEST_SYSTEMD_UNIT"
mkdir -p "$DEST_SYSTEMD_UNIT"
backup_if_diff "$SYSTEMD_UNIT" "$DEST_SYSTEMD_UNIT/xdg-desktop-portal-lunaris.service"
install -m 0644 "$SYSTEMD_UNIT" "$DEST_SYSTEMD_UNIT/"

echo "[4/5] Installing portal config to $DEST_PORTAL_CFG"
mkdir -p "$DEST_PORTAL_CFG"
backup_if_diff "$PORTAL_CONFIG" "$DEST_PORTAL_CFG/lunaris.portal"
install -m 0644 "$PORTAL_CONFIG" "$DEST_PORTAL_CFG/"

echo "[5/5] Installing systemd user-environment-generator"
mkdir -p "$DEST_ENV_GEN"
# Additive XDG_CURRENT_DESKTOP: keep existing values if any other
# session manager has set one, prepend `lunaris` so apps querying
# our identifier match first while wlroots-aware apps still see
# `wlroots` further down the colon-separated list.
cat > "$DEST_ENV_GEN/$ENV_GEN_NAME" <<'EOF'
#!/bin/sh
# Lunaris session marker. Frontend portal daemon's `UseIn=lunaris;`
# matcher reads XDG_CURRENT_DESKTOP to pick the right backend.
# Set additively so wlroots/GNOME-aware apps that fall back on a
# secondary identifier still find one.
existing="${XDG_CURRENT_DESKTOP:-}"
case ":$existing:" in
    *:lunaris:*) ;;
    *)
        if [ -n "$existing" ]; then
            echo "XDG_CURRENT_DESKTOP=lunaris:$existing"
        else
            echo "XDG_CURRENT_DESKTOP=lunaris:wlroots"
        fi
        ;;
esac
EOF
chmod 0755 "$DEST_ENV_GEN/$ENV_GEN_NAME"

# ── Post-install ───────────────────────────────────────────────

echo
echo "=== Install complete ==="
echo
echo "Next steps (run as your normal user, NOT root):"
echo "  1. Reload the user systemd manager so it picks up the new"
echo "     unit file under /usr/lib/systemd/user (D-Bus-activated"
echo "     services bind to the unit metadata at activation time;"
echo "     stale state without reload would launch the old binary):"
echo "       systemctl --user daemon-reload"
echo "  2. Restart the portal frontend so it re-reads .portal configs:"
echo "       systemctl --user restart xdg-desktop-portal"
echo "  3. Verify the backend is registered:"
echo "       busctl --user list | grep org.freedesktop.impl.portal.desktop.lunaris"
echo "  4. Log out / log back in so the environment generator runs"
echo "     and \$XDG_CURRENT_DESKTOP includes 'lunaris'."
echo
echo "Uninstall: see distro/uninstall-portal.sh (not yet shipped)."
