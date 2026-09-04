# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# A private session bus for a drive, so a producer that talks D-Bus can be driven
# against a real consumer without touching the session's own.
#
# WHY IT EXISTS. The file manager registers a progress job over the session bus,
# and the row it produces renders in the shell's notification popover - somewhere
# `drive-files.sh` cannot see from where it stands. So the job strand had no
# end-to-end coverage at all, and on 5 September that cost a real defect: a failed
# operation returned past its `finish` call and left the row up forever. It was
# found by READING, and reading is not a strategy.
#
# THE CONSUMER IS THE REAL ONE, not a stub. `arlen-notifyd` already owns
# `org.freedesktop.Notifications` and serves the JobViewServer at
# `/org/arlen/JobViewServer`; standing a fake in front of it would test my
# understanding of the interface rather than the interface.
#
# ON THE ADDRESS. The listen path is pinned in the config rather than read back
# from `--print-address`, which is the form `dev/integration` settled on: the
# address is then known before the daemon starts, so the caller waits for the
# SOCKET instead of parsing a stream. NB `dbus-send --address=` does not perform
# the Hello handshake and answers "Client tried to send a message other than
# Hello without being registered"; export DBUS_SESSION_BUS_ADDRESS and pass
# `--session` instead. That cost a diagnosis and reads as a daemon fault.

# Start a private session bus with its socket under $1. Echoes the address.
#
# Sets BUS_PID for the caller's cleanup trap. Permissive policy: any client may
# own a name and talk to any other, which is right for a bus with exactly the
# processes this script started on it.
start_private_bus() {
    _bus_dir="$1"
    _bus_sock="$_bus_dir/bus.sock"
    cat > "$_bus_dir/bus.conf" <<XML
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path=$_bus_sock</listen>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
XML
    dbus-daemon --config-file="$_bus_dir/bus.conf" --nofork --nopidfile >/dev/null 2>&1 &
    BUS_PID=$!
    for _ in $(seq 1 40); do
        [ -S "$_bus_sock" ] && { echo "unix:path=$_bus_sock"; return 0; }
        sleep 0.25
    done
    echo "!! the private bus never bound $_bus_sock" >&2
    return 1
}

# Whether $2 owns the name $1 on the bus at address $2... callers pass (name, addr).
#
# Asks the bus driver rather than sleeping and hoping: a daemon that logs "ready"
# may still have lost the name request, and those are different failures with the
# same appearance.
bus_owns_name() {
    DBUS_SESSION_BUS_ADDRESS="$2" dbus-send --session --dest=org.freedesktop.DBus \
        --type=method_call --print-reply /org/freedesktop/DBus \
        org.freedesktop.DBus.ListNames 2>/dev/null | grep -q "\"$1\""
}

# Wait until $1 owns a name on the bus at $2, or fail after ~15s.
wait_for_bus_name() {
    for _ in $(seq 1 60); do
        bus_owns_name "$1" "$2" && return 0
        sleep 0.25
    done
    echo "!! nothing claimed $1 on $2; nothing below is about that service" >&2
    return 1
}
