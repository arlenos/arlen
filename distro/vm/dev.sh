#!/usr/bin/env bash
# Lunaris dev environment launcher.
#
# Builds all daemons, copies them to the VM, and starts them in a tmux session.
# The VM must be running before calling this script.
#
# Usage: ./dev.sh
# Attach to existing session: ./dev.sh attach

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SSH_PORT=2222
SSH_OPTS="-p $SSH_PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -i $HOME/.ssh/id_rsa"
VM_USER=tim
VM_HOST=localhost
TMUX_SESSION=lunaris-dev

VM_PRODUCER_SOCKET="/tmp/lunaris-producer.sock"
VM_CONSUMER_SOCKET="/tmp/lunaris-consumer.sock"
VM_DB_PATH="/tmp/lunaris-events.db"
VM_GRAPH_PATH="/tmp/lunaris-graph"
VM_DAEMON_SOCKET="/tmp/lunaris-daemon.sock"
VM_SESSION_ID="dev-session-$(date +%s)"

ssh_vm() {
    ssh $SSH_OPTS "$VM_USER@$VM_HOST" "$@"
}

scp_to_vm() {
    scp -P $SSH_PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -i "$HOME/.ssh/id_rsa" "$1" "$VM_USER@$VM_HOST:$2"
}

build_all() {
    echo "==> Building event-bus"
    cargo build --manifest-path "$REPO_ROOT/event-bus/Cargo.toml"

    echo "==> Building knowledge"
    cargo build --manifest-path "$REPO_ROOT/knowledge/Cargo.toml"

    echo "==> Building kernel-layer eBPF"
    (cd "$REPO_ROOT/kernel-layer" && cargo +nightly build \
        -Z build-std=core \
        --target bpfel-unknown-none \
        -p kernel-layer-ebpf \
        --release)

    echo "==> Building kernel-layer daemon"
    (cd "$REPO_ROOT/kernel-layer" && cargo build -p kernel-layer)
}

copy_to_vm() {
    echo "==> Copying binaries to VM"
    scp_to_vm "$REPO_ROOT/event-bus/target/debug/event-bus" "~/event-bus"
    scp_to_vm "$REPO_ROOT/knowledge/target/debug/knowledge" "~/knowledge"
    scp_to_vm "$REPO_ROOT/kernel-layer/target/debug/kernel-layer" "~/kernel-layer"
    scp_to_vm "$REPO_ROOT/kernel-layer/target/bpfel-unknown-none/release/kernel-layer-ebpf" "~/kernel-layer-ebpf"
}

setup_vm() {
    echo "==> Setting up VM"
    ssh_vm "which tmux || sudo dnf install -y tmux -q"
    ssh_vm "mkdir -p /tmp"
}

start_tmux() {
    echo "==> Starting tmux session in VM"

    # Kill any existing session
    ssh_vm "tmux kill-session -t $TMUX_SESSION 2>/dev/null || true"

    # Create new session with event-bus in first pane
    ssh_vm "tmux new-session -d -s $TMUX_SESSION -x 220 -y 50 \
        'LUNARIS_PRODUCER_SOCKET=$VM_PRODUCER_SOCKET \
         LUNARIS_CONSUMER_SOCKET=$VM_CONSUMER_SOCKET \
         RUST_LOG=info \
         ./event-bus 2>&1 | tee /tmp/event-bus.log; bash'"

    # Split horizontally: knowledge in second pane
    sleep 1
    ssh_vm "tmux split-window -t $TMUX_SESSION -h \
        'sleep 1 && \
         LUNARIS_CONSUMER_SOCKET=$VM_CONSUMER_SOCKET \
         LUNARIS_DB_PATH=$VM_DB_PATH \
         LUNARIS_GRAPH_PATH=$VM_GRAPH_PATH \
         LUNARIS_DAEMON_SOCKET=$VM_DAEMON_SOCKET \
         RUST_LOG=info \
         ./knowledge 2>&1 | tee /tmp/knowledge.log; bash'"

    # Split vertically in right pane: kernel-layer in third pane
    sleep 1
    ssh_vm "tmux split-window -t $TMUX_SESSION -v \
        'sleep 2 && \
         LUNARIS_PRODUCER_SOCKET=$VM_PRODUCER_SOCKET \
         LUNARIS_SESSION_ID=$VM_SESSION_ID \
         RUST_LOG=info \
         sudo -E ./kernel-layer 2>&1 | tee /tmp/kernel-layer.log; bash'"

    # Set pane titles
    ssh_vm "tmux select-pane -t $TMUX_SESSION:0.0 -T event-bus"
    ssh_vm "tmux select-pane -t $TMUX_SESSION:0.1 -T knowledge"
    ssh_vm "tmux select-pane -t $TMUX_SESSION:0.2 -T kernel-layer"
}

attach() {
    echo "==> Attaching to tmux session in VM"
    echo "    Detach with: Ctrl-B D"
    TERM=xterm-256color ssh $SSH_OPTS -t "$VM_USER@$VM_HOST" "tmux attach-session -t $TMUX_SESSION"
}

case "${1:-run}" in
    run)
        build_all
        copy_to_vm
        setup_vm
        start_tmux
        echo ""
        echo "Dev environment started. Attaching..."
        sleep 2
        attach
        ;;
    build)
        build_all
        copy_to_vm
        ;;
    attach)
        attach
        ;;
    logs)
        echo "==> event-bus log:"
        ssh_vm "tail -20 /tmp/event-bus.log"
        echo "==> knowledge log:"
        ssh_vm "tail -20 /tmp/knowledge.log"
        echo "==> kernel-layer log:"
        ssh_vm "tail -20 /tmp/kernel-layer.log"
        ;;
    stop)
        ssh_vm "tmux kill-session -t $TMUX_SESSION 2>/dev/null || true"
        echo "Dev environment stopped."
        ;;
    *)
        echo "Usage: $0 {run|build|attach|logs|stop}"
        exit 1
        ;;
esac
