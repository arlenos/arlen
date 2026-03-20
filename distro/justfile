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
    echo "TODO: start event-bus and knowledge daemons"

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
