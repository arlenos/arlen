#!/usr/bin/env bash
# Test script: verifies that desktop-shell Top Bar reacts to window.focused events.
#
# What it does:
# 1. Starts the Event Bus
# 2. Sends a synthetic window.focused event
# 3. desktop-shell must be running separately (cargo tauri dev)
#
# Usage:
#   Terminal 1: LUNARIS_CONSUMER_SOCKET=/tmp/test-consumer.sock cargo tauri dev
#   Terminal 2: ./test-shell.sh

set -euo pipefail

PRODUCER_SOCKET=/tmp/test-producer.sock
CONSUMER_SOCKET=/tmp/test-consumer.sock
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

echo "==> Starting Event Bus"
LUNARIS_PRODUCER_SOCKET=$PRODUCER_SOCKET \
LUNARIS_CONSUMER_SOCKET=$CONSUMER_SOCKET \
RUST_LOG=info \
"$HOME/Repositories/lunaris-sys/event-bus/target/debug/event-bus" &
BUS_PID=$!

echo "==> Waiting for desktop-shell to connect... press Enter when ready"
read -r
echo "==> Sending synthetic window.focused event"
# Build a minimal protobuf Event and send it length-prefixed to the producer socket
python3 << 'PYEOF'
import socket
import struct
import json

# Minimal protobuf encoding for our Event message:
# field 1 (id): string
# field 2 (type): string  
# field 6 (pid): uint32
# field 7 (session_id): string
# field 8 (payload): bytes (JSON)

def encode_string(field_num, value):
    encoded = value.encode('utf-8')
    tag = (field_num << 3) | 2  # wire type 2 = length-delimited
    return bytes([tag]) + encode_varint(len(encoded)) + encoded

def encode_varint(value):
    result = []
    while value > 0x7f:
        result.append((value & 0x7f) | 0x80)
        value >>= 7
    result.append(value)
    return bytes(result)

payload = json.dumps({"app_id": "org.mozilla.firefox", "title": "Firefox"}).encode()

msg = b""
msg += encode_string(1, "test-event-001")      # id
msg += encode_string(2, "window.focused")       # type
# timestamp as int64 varint (field 3, wire type 0)
msg += bytes([0x18]) + encode_varint(1000000000)        # timestamp
msg += encode_string(4, "compositor")           # source
msg += encode_string(6, "test-session")         # session_id
msg += encode_string(7, payload.decode())       # payload as string for simplicity

# Length prefix
frame = struct.pack(">I", len(msg)) + msg

sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
sock.connect("/tmp/test-producer.sock")
sock.sendall(frame)
sock.close()
print("Sent window.focused for org.mozilla.firefox")
PYEOF

echo ""
echo "==> Check if Top Bar shows 'org.mozilla.firefox'"
echo "    Press Ctrl+C to stop"

wait $BUS_PID
