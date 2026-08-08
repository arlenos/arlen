#!/bin/bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Drive the session undo service on a PRIVATE bus and read what it answers.
#
# Run:  dbus-run-session -- dev/scripts/probe-undo-service.sh
#
# A private bus, never the developer's own session: this daemon takes
# `org.arlen.Undo1` as sole owner and refuses to start if the name is held, so
# probing on a live session would either fail or take the name from the real one.
#
# What it proves, and it is worth being exact: `busctl` is not a user surface, so
# its identity does not resolve and BOTH methods must refuse. Seeing the refusal
# is the point - a run where they answered would mean the admission gate is not
# doing anything. The journal lines underneath are the other half: a refusal that
# nothing records is a refusal nobody can diagnose.
set -u
export XDG_RUNTIME_DIR=$(mktemp -d)
export RUST_LOG=info
target/debug/arlen-undod >/tmp/undod.log 2>&1 &
UP=$!
sleep 3
echo "--- owns the name? ---"
busctl --user list 2>/dev/null | grep -i "org.arlen.Undo1" || echo "  NOT on the bus"
echo "--- Recent ---"
timeout 10 busctl --user call org.arlen.Undo1 /org/arlen/Undo1 org.arlen.Undo1 Recent 2>&1 | head -3
echo "--- Enact with a nonsense id ---"
timeout 10 busctl --user call org.arlen.Undo1 /org/arlen/Undo1 org.arlen.Undo1 Enact s "no-such-op" 2>&1 | head -3
echo "--- journal ---"
tail -5 /tmp/undod.log
kill $UP 2>/dev/null
