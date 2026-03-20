# Lunaris top-level task runner
# Requires: just, cargo, qemu, mkosi

# Start Event Bus and Graph Daemon in the current session
dev:
    echo "TODO: start event-bus and graph-daemon"

# Run unit tests across all repos in dependency order
test:
    cargo test --manifest-path ../event-bus/Cargo.toml
    cargo test --manifest-path ../sdk/Cargo.toml
    cargo test --manifest-path ../knowledge/Cargo.toml

# Verify all repos are on the pinned dependency versions
check-deps:
    echo "TODO: implement dependency drift check against workspace-deps.toml"

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
