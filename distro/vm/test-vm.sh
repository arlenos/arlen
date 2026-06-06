#!/usr/bin/env bash
# Manual VM smoke test: verifies that a real eBPF file.opened event
# lands in SQLite via the full stack.
# Run with: ./test-vm.sh
# Requires: VM running (./setup-vm.sh start)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/dev.sh" build 2>/dev/null || true

SSH_OPTS="-p 2222 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -i $HOME/.ssh/id_rsa"

echo "==> Starting full stack in VM"
ssh $SSH_OPTS tim@localhost "
  pkill event-bus || true
  pkill knowledge || true
  pkill kernel-layer || true
  sleep 1

  LUNARIS_PRODUCER_SOCKET=/tmp/test-producer.sock \
  LUNARIS_CONSUMER_SOCKET=/tmp/test-consumer.sock \
  RUST_LOG=error ./event-bus &

  sleep 1

  LUNARIS_CONSUMER_SOCKET=/tmp/test-consumer.sock \
  LUNARIS_DB_PATH=/tmp/test-events.db \
  LUNARIS_GRAPH_PATH=/tmp/test-graph \
  LUNARIS_DAEMON_SOCKET=/tmp/test-daemon.sock \
  RUST_LOG=error ./knowledge &

  sleep 1

  sudo LUNARIS_PRODUCER_SOCKET=/tmp/test-producer.sock \
  LUNARIS_SESSION_ID=smoke-test \
  RUST_LOG=error ./kernel-layer &

  sleep 3
  cat /etc/hostname
  sleep 2

  pkill kernel-layer || true
  sleep 1

  COUNT=\$(sqlite3 /tmp/test-events.db 'SELECT COUNT(*) FROM events WHERE type=\"file.opened\"' 2>/dev/null || echo 0)
  echo \"file.opened events in SQLite: \$COUNT\"
  if [ \"\$COUNT\" -gt 0 ]; then
    echo 'PASS: eBPF events landed in SQLite'
    exit 0
  else
    echo 'FAIL: no events found in SQLite'
    exit 1
  fi
"
