#!/usr/bin/env bash
# Install aya toolchain on the host (your EndeavourOS machine)
# Run once after rustup is already installed

set -euo pipefail

echo "==> Installing required rustup components"
rustup component add rust-src
rustup target add x86_64-unknown-linux-musl

echo "==> Installing bpf-linker"
# bpf-linker requires LLVM. On Arch/EndeavourOS it uses the system LLVM.
cargo install bpf-linker

echo "==> Installing cargo-generate (for aya project template)"
cargo install cargo-generate

echo "==> Verifying host LLVM"
llvm-config --version || echo "WARNING: llvm-config not found, install llvm package"

echo ""
echo "All done. Verify with:"
echo "  rustup target list --installed | grep bpf"
echo "  cargo bpf --version  (not needed, bpf-linker is used directly)"
echo ""
echo "Next: set up the VM with ./setup-vm.sh setup"
