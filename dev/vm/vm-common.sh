# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# What the eBPF dev VM's two scripts must agree on, in one place.
#
# `setup-vm.sh` put one key into the VM and `dev.sh` logged in with another:
# setup preferred `id_ed25519.pub`, dev.sh hard-coded `id_rsa`, so on any machine
# holding both the login could never succeed. The user was hard-coded the same way
# (`tim`, in the cloud-init file and again in dev.sh). Both are chosen here once
# and sourced by both scripts.

# The guest user. The person running the scripts unless told otherwise.
VM_USER="${ARLEN_VM_USER:-$USER}"

# The private key used to log in; its `.pub` is what setup puts into the guest.
# ed25519 first, then rsa, the same order setup always used.
vm_key() {
    if [ -n "${ARLEN_VM_KEY:-}" ]; then
        echo "$ARLEN_VM_KEY"
        return
    fi
    local k
    for k in "$HOME/.ssh/id_ed25519" "$HOME/.ssh/id_rsa"; do
        if [ -f "$k" ] && [ -f "$k.pub" ]; then
            echo "$k"
            return
        fi
    done
    echo "ERROR: no SSH key pair at ~/.ssh/id_ed25519 or ~/.ssh/id_rsa (or set ARLEN_VM_KEY)" >&2
    return 1
}

SSH_PORT="${ARLEN_VM_SSH_PORT:-2222}"
