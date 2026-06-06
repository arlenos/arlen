#!/bin/bash
# Lunaris portal — undo the per-user dev-mode setup.
#
# Removes the user-local D-Bus service shim. Does not delete
# build artefacts under target/ — those are useful between
# dev sessions and removing them would force a slow rebuild.

set -euo pipefail

USER_DBUS_SVC="$HOME/.local/share/dbus-1/services"
DBUS_SVC_NAME="org.freedesktop.impl.portal.desktop.lunaris.service"

echo "=== Lunaris portal dev teardown ==="

if [ -e "$USER_DBUS_SVC/$DBUS_SVC_NAME" ]; then
    rm -f "$USER_DBUS_SVC/$DBUS_SVC_NAME"
    echo "Removed $USER_DBUS_SVC/$DBUS_SVC_NAME"
else
    echo "No dev service file found, nothing to remove."
fi

echo
echo "Restart the portal frontend to clear the registration:"
echo "  systemctl --user restart xdg-desktop-portal"
