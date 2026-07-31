#!/bin/bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Arlen portal - undo what install-portal.sh put under /usr.
#
# The installer has pointed at this file since it was written, with a
# "(not yet shipped)" next to it. This is that file.
#
# It removes exactly the six paths the installer creates and nothing
# else. In particular it does not remove the directories they live in:
# /usr/share/dbus-1/services and /usr/lib/systemd/user belong to the
# distribution and hold other packages' files. Only /usr/lib/arlen/libexec
# is ours, and even that is left in place because other Arlen components
# install there too.
#
# Backups the installer made (`*.bak.<stamp>`, written only when it would
# have overwritten a file whose content differed) are listed rather than
# deleted: they exist because someone's local edit was about to be lost,
# so removing them silently is the one thing this script must not do.

set -euo pipefail

DEST_LIBEXEC="/usr/lib/arlen/libexec"
DEST_DBUS_SVC="/usr/share/dbus-1/services"
DEST_SYSTEMD_UNIT="/usr/lib/systemd/user"
DEST_PORTAL_CFG="/usr/share/xdg-desktop-portal/portals"
DEST_ENV_GEN="/usr/lib/systemd/user-environment-generators"
ENV_GEN_NAME="30-arlen"

INSTALLED=(
    "$DEST_LIBEXEC/xdg-desktop-portal-arlen"
    "$DEST_LIBEXEC/xdg-desktop-portal-arlen-picker"
    "$DEST_DBUS_SVC/org.freedesktop.impl.portal.desktop.arlen.service"
    "$DEST_SYSTEMD_UNIT/xdg-desktop-portal-arlen.service"
    "$DEST_PORTAL_CFG/arlen.portal"
    "$DEST_ENV_GEN/$ENV_GEN_NAME"
)

if [ "$(id -u)" -ne 0 ]; then
    echo "Re-executing under sudo for /usr writes..."
    exec sudo "$0" "$@"
fi

echo "=== Arlen portal uninstall ==="

# Stop the running daemon before removing the unit, or systemd keeps a
# service it can no longer describe. The portal runs in the user session,
# so this is addressed to the invoking user rather than to root.
if [ -n "${SUDO_USER:-}" ]; then
    echo "[1/3] Stopping the user service"
    sudo -u "$SUDO_USER" XDG_RUNTIME_DIR="/run/user/$(id -u "$SUDO_USER")" \
        systemctl --user stop xdg-desktop-portal-arlen.service 2>/dev/null ||
        echo "  (not running)"
else
    echo "[1/3] Skipping service stop: no SUDO_USER, run as the session user"
fi

echo "[2/3] Removing installed files"
removed=0
for f in "${INSTALLED[@]}"; do
    if [ -e "$f" ]; then
        rm -f "$f"
        echo "  removed $f"
        removed=$((removed + 1))
    fi
done
[ "$removed" -eq 0 ] && echo "  nothing to remove; the portal was not installed here"

echo "[3/3] Reloading the D-Bus and systemd views"
if [ -n "${SUDO_USER:-}" ]; then
    sudo -u "$SUDO_USER" XDG_RUNTIME_DIR="/run/user/$(id -u "$SUDO_USER")" \
        systemctl --user daemon-reload 2>/dev/null || true
fi

leftovers=$(find "$DEST_LIBEXEC" "$DEST_DBUS_SVC" "$DEST_SYSTEMD_UNIT" \
    "$DEST_PORTAL_CFG" "$DEST_ENV_GEN" -maxdepth 1 -name '*arlen*.bak.*' 2>/dev/null || true)
if [ -n "$leftovers" ]; then
    echo
    echo "Backups the installer made are still here. They hold local edits it"
    echo "was about to overwrite, so decide yourself rather than have this"
    echo "script delete them:"
    printf '  %s\n' $leftovers
fi

echo
echo "Done. The per-user dev shim, if you used dev-portal-setup.sh, is"
echo "removed separately by dev-portal-teardown.sh."
