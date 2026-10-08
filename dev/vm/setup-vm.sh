#!/usr/bin/env bash
# Arlen eBPF dev VM setup and start script
# Run once: ./setup-vm.sh setup
# Run after: ./setup-vm.sh start

#
# DEBIAN TRIXIE, the release the image is built from (`dev/mkosi/mkosi.conf`).
# This was a Fedora 41 cloud image with Fedora package names and `dnf`, from the
# days the target was Fedora; the kernel layer is meant to run on the kernel and
# userland the image ships, so the dev VM follows the image.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
. "$SCRIPT_DIR/vm-common.sh"

VM_DIR="$HOME/vms/arlen-ebpf"
DISK_IMG="$VM_DIR/debian-ebpf.qcow2"
CLOUD_IMG="$VM_DIR/debian-base.qcow2"
CLOUD_INIT_IMG="$VM_DIR/cloud-init.iso"
DEBIAN_URL="https://cloud.debian.org/images/cloud/trixie/latest/debian-13-genericcloud-amd64.qcow2"
VM_RAM=4096
VM_CPUS=4
VM_DISK_SIZE=20G

setup() {
    echo "==> Creating VM directory"
    mkdir -p "$VM_DIR"

    echo "==> Downloading the Debian trixie cloud image"
    if [ ! -f "$CLOUD_IMG" ]; then
        curl -L --fail -o "$CLOUD_IMG" "$DEBIAN_URL"
    else
        echo "    Already downloaded, skipping"
    fi

    echo "==> Creating VM disk (${VM_DISK_SIZE})"
    qemu-img create -f qcow2 -F qcow2 -b "$CLOUD_IMG" "$DISK_IMG" "$VM_DISK_SIZE"

    echo "==> Injecting SSH public key and user into cloud-init config"
    KEY=$(vm_key)
    PUBKEY=$(cat "$KEY.pub")
    sed -e "s|REPLACE_WITH_YOUR_PUBLIC_KEY|$PUBKEY|" \
        -e "s|REPLACE_WITH_USER|$VM_USER|g" \
        "$SCRIPT_DIR/cloud-init-user-data.yaml" > "$VM_DIR/user-data"

    cat > "$VM_DIR/meta-data" << 'EOF'
instance-id: arlen-ebpf-dev-01
local-hostname: arlen-ebpf-dev
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
    echo "==> Starting Arlen eBPF dev VM"
    echo "    SSH will be available at: ssh -p $SSH_PORT $VM_USER@localhost"
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
        -i "$(vm_key)" \
        "$VM_USER@localhost" "$@"
}

case "${1:-}" in
    setup) setup ;;
    start) start ;;
    ssh)   shift; ssh_vm "$@" ;;
    *)
        echo "Usage: $0 {setup|start|ssh}"
        echo ""
        echo "  setup  - Download the Debian trixie image and prepare the VM disk"
        echo "  start  - Start the VM (KVM accelerated, headless)"
        echo "  ssh    - SSH into the running VM"
        exit 1
        ;;
esac
