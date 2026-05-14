#!/usr/bin/env bash
# reset-sway-portals.sh — restore standard XDG portals after Lunaris dev work.
#
# After developing on Lunaris (xdg-desktop-portal-lunaris dev-mode setup,
# nested compositor sessions, etc.) the user-level systemd env and D-Bus
# state can leave Sway pointing at a non-existent or unwanted Lunaris
# portal backend. Symptoms: file pickers don't open, screensharing fails,
# `flatpak run` apps complain about missing portal interfaces.
#
# This script does the full reset:
#   1. Kills lingering xdg-desktop-portal-lunaris processes.
#   2. Removes the per-user D-Bus service shim that dev-portal-setup.sh
#      installs (mirrors dev-portal-teardown.sh).
#   3. Backs up any user-level portal-config files that mention lunaris.
#   4. Resets the polluted XDG_CURRENT_DESKTOP / XDG_DATA_DIRS values in
#      the systemd user environment back to plain `sway`.
#   5. Restarts xdg-desktop-portal + the gtk/wlr backends.
#   6. Verifies the standard portal is reachable.
#
# Idempotent: safe to run multiple times. Only modifies user-scope state
# (no sudo).
#
# Usage:
#   ./distro/reset-sway-portals.sh

set -euo pipefail

red()    { printf '\033[31m%s\033[0m\n' "$*"; }
green()  { printf '\033[32m%s\033[0m\n' "$*"; }
yellow() { printf '\033[33m%s\033[0m\n' "$*"; }
note()   { printf '    %s\n' "$*"; }
step()   { printf '\n→ %s\n' "$*"; }

USER_DBUS_SVC="$HOME/.local/share/dbus-1/services"
LUNARIS_DBUS_SVC="$USER_DBUS_SVC/org.freedesktop.impl.portal.desktop.lunaris.service"
USER_PORTAL_DIR="$HOME/.config/xdg-desktop-portal"
TS="$(date +%Y%m%d-%H%M%S)"

# ── 1. Stop lingering Lunaris portal processes ─────────────────
step "1/6  Stopping lingering Lunaris portal processes"
killed_any=0
for pat in 'xdg-desktop-portal-lunaris-picker' 'xdg-desktop-portal-lunaris'; do
    if pgrep -af "$pat" >/dev/null 2>&1; then
        pgrep -af "$pat" | sed 's/^/    found: /'
        pkill -TERM -f "$pat" 2>/dev/null || true
        killed_any=1
    fi
done
if [ $killed_any -eq 1 ]; then
    sleep 1
    pkill -KILL -f 'xdg-desktop-portal-lunaris' 2>/dev/null || true
    green "  killed Lunaris portal binaries"
else
    note "(none running)"
fi

# ── 2. Remove dev D-Bus service shim ───────────────────────────
step "2/6  Removing dev-mode D-Bus service shim"
if [ -e "$LUNARIS_DBUS_SVC" ]; then
    rm -f "$LUNARIS_DBUS_SVC"
    green "  removed: $LUNARIS_DBUS_SVC"
else
    note "(none to remove)"
fi

# ── 3. Quarantine user-level portal config that references Lunaris ─
step "3/6  Quarantining Lunaris-referencing portal configs"
quarantined=0
if [ -d "$USER_PORTAL_DIR" ]; then
    for f in "$USER_PORTAL_DIR"/portals.conf \
             "$USER_PORTAL_DIR"/sway-portals.conf \
             "$USER_PORTAL_DIR"/lunaris-portals.conf; do
        if [ -f "$f" ]; then
            if grep -qi 'lunaris' "$f"; then
                mv "$f" "${f}.lunaris-reset.${TS}.bak"
                yellow "  quarantined: $f"
                note "(backup: ${f}.lunaris-reset.${TS}.bak)"
                quarantined=1
            else
                note "kept (no lunaris reference): $f"
            fi
        fi
    done
fi
[ $quarantined -eq 0 ] && note "(none needed quarantine)"

# ── 4. Reset polluted systemd user environment ─────────────────
step "4/6  Resetting systemd user XDG_CURRENT_DESKTOP / XDG_DATA_DIRS"
current_xdg=$(systemctl --user show-environment 2>/dev/null \
              | awk -F= '/^XDG_CURRENT_DESKTOP=/ {print $2}' || true)
current_dirs=$(systemctl --user show-environment 2>/dev/null \
              | awk -F= '/^XDG_DATA_DIRS=/ {print $2}' || true)

note "current XDG_CURRENT_DESKTOP=${current_xdg:-(unset)}"
note "current XDG_DATA_DIRS=${current_dirs:-(unset)}"

needs_reset=0
case "$current_xdg" in
    *lunaris*) needs_reset=1 ;;
esac
case "$current_dirs" in
    *lunaris-sys*|*xdg-desktop-portal-lunaris*) needs_reset=1 ;;
esac

if [ $needs_reset -eq 1 ]; then
    systemctl --user unset-environment XDG_CURRENT_DESKTOP XDG_DATA_DIRS 2>/dev/null || true
    systemctl --user set-environment "XDG_CURRENT_DESKTOP=sway"
    systemctl --user set-environment \
        "XDG_DATA_DIRS=/usr/local/share:/usr/share:/var/lib/flatpak/exports/share:$HOME/.local/share/flatpak/exports/share"
    green "  reset to: XDG_CURRENT_DESKTOP=sway"
    yellow "  Note: a clean log-out + log-in to Sway is the most reliable"
    yellow "  fix for the user-env path. This script unblocks the daemon"
    yellow "  side, but shell-spawned children inherit from your login."
else
    green "  already clean"
fi

# ── 5. Restart xdg-desktop-portal frontend + backends ──────────
step "5/6  Restarting xdg-desktop-portal user units"
units_to_restart=(
    xdg-desktop-portal.service
    xdg-desktop-portal-gtk.service
    xdg-desktop-portal-wlr.service
)
for unit in "${units_to_restart[@]}"; do
    if systemctl --user list-unit-files "$unit" 2>/dev/null | grep -q "$unit"; then
        systemctl --user reset-failed "$unit" 2>/dev/null || true
        systemctl --user restart "$unit" 2>/dev/null \
            && note "restarted: $unit" \
            || yellow "  could not restart: $unit (will lazy-start on next D-Bus call)"
    fi
done

# ── 6. Verify ──────────────────────────────────────────────────
step "6/6  Verifying standard portal availability"
sleep 1

verified=true

if pgrep -f 'xdg-desktop-portal-lunaris' >/dev/null 2>&1; then
    red "  Lunaris portal still running after kill — investigate"
    verified=false
fi

if busctl --user --no-pager list 2>/dev/null \
   | grep -q 'org\.freedesktop\.portal\.Desktop'; then
    green "  D-Bus name org.freedesktop.portal.Desktop reachable"
else
    yellow "  portal frontend not yet active (lazy-start)"
    note "trying to wake it now..."
    gdbus call --session \
        --dest org.freedesktop.portal.Desktop \
        --object-path /org/freedesktop/portal/desktop \
        --method org.freedesktop.DBus.Introspectable.Introspect \
        >/dev/null 2>&1 || true
    sleep 1
    if pgrep -f 'xdg-desktop-portal' >/dev/null 2>&1; then
        green "  portal up:"
        pgrep -af 'xdg-desktop-portal' | head -3 | sed 's/^/    /'
    else
        red "  portal still not running"
        verified=false
    fi
fi

# Probe interface routing: which backend would actually answer?
backend_owner=$(busctl --user --no-pager get-property \
    org.freedesktop.portal.Desktop \
    /org/freedesktop/portal/desktop \
    org.freedesktop.DBus.Properties \
    "version" 2>/dev/null | head -1 || true)

if [ -n "$backend_owner" ]; then
    note "portal frontend responding"
fi

echo
if $verified; then
    green "✓ Sway XDG portals reset complete."
else
    red "✗ Reset partially completed — some checks failed."
fi

echo
echo "If apps still misbehave (file picker won't open, screencast fails):"
note "1. log out of Sway and back in (cleanest reset of XDG_CURRENT_DESKTOP)"
note "2. journalctl --user -u xdg-desktop-portal -e --since '2 minutes ago'"
note "3. test directly: /usr/lib/xdg-desktop-portal --replace --verbose"
note "4. re-run this script"
