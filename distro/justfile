# Lunaris top-level task runner
# Requires: just, cargo, qemu, mkosi

REPOS := "../event-bus ../sdk ../knowledge"

# Build all components in dependency order
build:
    cargo build --manifest-path ../event-bus/Cargo.toml
    cargo build --manifest-path ../sdk/Cargo.toml
    cargo build --manifest-path ../knowledge/Cargo.toml

# Run all tests across all repos in dependency order
test: build
    cargo test --manifest-path ../event-bus/Cargo.toml
    cargo test --manifest-path ../sdk/Cargo.toml
    cargo test --manifest-path ../knowledge/Cargo.toml
    cargo test --manifest-path ../knowledge/Cargo.toml --test event_pipeline

# Run clippy across all repos
lint:
    cargo clippy --manifest-path ../event-bus/Cargo.toml --all-targets --all-features -- -D warnings
    cargo clippy --manifest-path ../sdk/Cargo.toml --all-targets --all-features -- -D warnings
    cargo clippy --manifest-path ../knowledge/Cargo.toml --all-targets --all-features -- -D warnings

# Check dependency versions are in sync with workspace-deps.toml
check-deps:
    echo "TODO: implement dependency drift check against workspace-deps.toml"

# Start Event Bus and Graph Daemon in the current session
dev:
    vm/dev.sh run

# Build production ISO via mkosi
build-iso:
    echo "TODO: mkosi build"

# Build developer ISO with SSH and debug symbols
build-dev:
    echo "TODO: mkosi build --target developer"

# Boot the developer ISO in QEMU with KVM acceleration
vm:
    echo "TODO: qemu launch"

# Run the shell inside the current Wayland session (no VM)
vm-nested:
    echo "TODO: nested compositor launch"

# Boot with GDB server exposed on port 1234
vm-debug:
    echo "TODO: qemu launch with -s -S"

# Run graph-scale benchmarks against synthetic datasets
bench:
    cargo bench --manifest-path ../knowledge/Cargo.toml

# Run upstream cosmic-comp rebase check
compositor-rebase-check:
    echo "TODO: git fetch upstream && git rebase upstream/main"

# DISPLAY="" forces the Winit/Wayland backend; the X11 backend does not
# advertise zwlr_layer_shell_v1 to gtk-layer-shell so the shell would lose
# its top-bar surface.
# Start compositor nested in current Wayland session.
compositor:
    cd ../compositor && DISPLAY="" WAYLAND_DISPLAY=wayland-1 cargo run --bin cosmic-comp

# Start desktop-shell against a running compositor (auto-detects WAYLAND_DISPLAY).
shell:
    #!/usr/bin/env bash
    set -euo pipefail
    socket=$(ls -t "$XDG_RUNTIME_DIR"/wayland-* 2>/dev/null | grep -v '\.lock$' | head -1)
    if [ -z "$socket" ]; then
        echo "no wayland-* socket in $XDG_RUNTIME_DIR — start the compositor first ('just compositor')" >&2
        exit 1
    fi
    display=$(basename "$socket")
    echo "using WAYLAND_DISPLAY=$display"
    cd ../desktop-shell && WAYLAND_DISPLAY="$display" cargo tauri dev

# Logs land in ../logs/. Detach with Ctrl+B d, reattach with
# `tmux attach -t lunaris`.
# Full-stack dev on host (event-bus + knowledge + notifyd + compositor + shell) in tmux.
dev-stack:
    bash start-dev.sh

# Use --lib only because the test-client binary has optional deps.
# Run compositor library tests.
test-compositor:
    cd ../compositor && cargo test --lib

# Run desktop-shell rust + svelte checks.
test-shell:
    cd ../desktop-shell/src-tauri && cargo test
    cd ../desktop-shell && npx svelte-check --threshold error --no-tsconfig

# Format all Rust crates.
fmt:
    cargo fmt --manifest-path ../event-bus/Cargo.toml --all
    cargo fmt --manifest-path ../sdk/Cargo.toml --all
    cargo fmt --manifest-path ../knowledge/Cargo.toml --all
    cargo fmt --manifest-path ../compositor/Cargo.toml --all
    cargo fmt --manifest-path ../notification-daemon/Cargo.toml --all
    cargo fmt --manifest-path ../desktop-shell/src-tauri/Cargo.toml --all
    cargo fmt --manifest-path ../installd/Cargo.toml --all
    cargo fmt --manifest-path ../forage/Cargo.toml --all
    cargo fmt --manifest-path ../kernel-layer/Cargo.toml --all || echo "kernel-layer fmt skipped (eBPF target needs nightly)"

# Compositor uses --lib because the test-client binary has optional deps.
# Cheap typecheck across all crates.
check:
    cargo check --manifest-path ../event-bus/Cargo.toml
    cargo check --manifest-path ../sdk/Cargo.toml
    cargo check --manifest-path ../knowledge/Cargo.toml
    cargo check --manifest-path ../compositor/Cargo.toml --lib
    cargo check --manifest-path ../notification-daemon/Cargo.toml
    cargo check --manifest-path ../desktop-shell/src-tauri/Cargo.toml
    cargo check --manifest-path ../installd/Cargo.toml
    cargo check --manifest-path ../forage/Cargo.toml
