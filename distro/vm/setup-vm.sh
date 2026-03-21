#!/usr/bin/env bash
# Lunaris eBPF dev VM setup and start script
# Run once: ./setup-vm.sh setup
# Run after: ./setup-vm.sh start

set -euo pipefail

VM_DIR="$HOME/vms/lunaris-ebpf"
DISK_IMG="$VM_DIR/fedora-ebpf.qcow2"
CLOUD_IMG="$VM_DIR/fedora-base.qcow2"
CLOUD_INIT_IMG="$VM_DIR/cloud-init.iso"
FEDORA_URL="https://download.fedoraproject.org/pub/fedora/linux/releases/41/Cloud/x86_64/images/Fedora-Cloud-Base-Generic-41-1.4.x86_64.qcow2"
SSH_PORT=2222
VM_RAM=4096
VM_CPUS=4
VM_DISK_SIZE=20G

setup() {
    echo "==> Creating VM directory"
    mkdir -p "$VM_DIR"

    echo "==> Downloading Fedora 41 Cloud image"
    if [ ! -f "$CLOUD_IMG" ]; then
        curl -L -o "$CLOUD_IMG" "$FEDORA_URL"
    else
        echo "    Already downloaded, skipping"
    fi

    echo "==> Creating VM disk (${VM_DISK_SIZE})"
    qemu-img create -f qcow2 -F qcow2 -b "$CLOUD_IMG" "$DISK_IMG" "$VM_DISK_SIZE"

    echo "==> Injecting SSH public key into cloud-init config"
    PUBKEY=$(cat "$HOME/.ssh/id_ed25519.pub" 2>/dev/null || cat "$HOME/.ssh/id_rsa.pub" 2>/dev/null || echo "")
    if [ -z "$PUBKEY" ]; then
        echo "ERROR: No SSH public key found at ~/.ssh/id_ed25519.pub or ~/.ssh/id_rsa.pub"
        exit 1
    fi

    SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
    sed "s|REPLACE_WITH_YOUR_PUBLIC_KEY|$PUBKEY|" \
        "$SCRIPT_DIR/cloud-init-user-data.yaml" > "$VM_DIR/user-data"

    cat > "$VM_DIR/meta-data" << 'EOF'
instance-id: lunaris-ebpf-dev-01
local-hostname: lunaris-ebpf-dev
EOF

    echo "==> Creating cloud-init ISO"
    mkisofs -output "$CLOUD_INIT_IMG" \
        -volid cidata \
        -joliet \
        -rock \
        "$VM_DIR/user-data" \
        "$VM_DIR/meta-data"

    echo ""
    echo "Setup complete. Run: ./setup-vm.sh start"
}

start() {
    echo "==> Starting Lunaris eBPF dev VM"
    echo "    SSH will be available at: ssh -p $SSH_PORT tim@localhost"
    echo "    First boot takes ~60 seconds for cloud-init to finish"
    echo ""

    qemu-system-x86_64 \
        -enable-kvm \
        -cpu host \
        -smp "$VM_CPUS" \
        -m "$VM_RAM" \
        -drive file="$DISK_IMG",format=qcow2,if=virtio \
        -drive file="$CLOUD_INIT_IMG",format=raw,if=virtio \
        -net nic,model=virtio \
        -net user,hostfwd=tcp::"$SSH_PORT"-:22 \
        -nographic \
        -serial mon:stdio
}

ssh_vm() {
    ssh -p "$SSH_PORT" \
        -o StrictHostKeyChecking=no \
        -o UserKnownHostsFile=/dev/null \
        tim@localhost "$@"
}

case "${1:-}" in
    setup) setup ;;
    start) start ;;
    ssh)   shift; ssh_vm "$@" ;;
    *)
        echo "Usage: $0 {setup|start|ssh}"
        echo ""
        echo "  setup  - Download Fedora image and prepare VM disk"
        echo "  start  - Start the VM (KVM accelerated, headless)"
        echo "  ssh    - SSH into the running VM"
        exit 1
        ;;
esac
