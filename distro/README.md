# distro

Build system, integration tests, and development tooling for Lunaris. This repo does not contain application code; it is the glue that holds everything together.

## What's here

```
distro/
├── justfile              build, test, lint recipes for the whole project
├── vm/
│   ├── setup-vm.sh       download Fedora Cloud image and create QEMU VM
│   ├── dev.sh            build all daemons, copy to VM, start tmux dev session
│   ├── test-vm.sh        smoke test: verify eBPF events land in SQLite
│   ├── install-aya-toolchain.sh  one-time host setup for eBPF development
│   └── cloud-init-user-data.yaml  VM provisioning config
└── tests/
    ├── event_pipeline.rs      integration test: synthetic event → SQLite
    └── integration_compositor.rs  integration test: compositor → Event Bus → SQLite
```

## Development workflow

The eBPF component (kernel-layer) must run in a VM because kernel bugs can destabilize the host. The VM also serves as the simulated target system: all daemons run together in the VM, mirroring the production setup.

**First time setup:**

```bash
# Install QEMU
sudo pacman -S qemu-system-x86 qemu-img cdrtools  # Arch/EndeavourOS

# Install aya toolchain (for eBPF development)
./vm/install-aya-toolchain.sh

# Create and start VM
./vm/setup-vm.sh setup
./vm/setup-vm.sh start
```

**Daily development:**

```bash
just dev       # build everything, copy to VM, open tmux session with all daemons
just test      # run all tests across all repos
just lint      # clippy across all repos
```

Inside the tmux session: left pane is event-bus, top-right is knowledge, bottom-right is kernel-layer. Detach with `Ctrl-B D`. Reattach with `just dev attach`.

## Integration tests

The tests in `tests/` start real daemon processes as subprocesses and verify end-to-end behaviour. They are slower than unit tests and require the binaries to be built first.

```bash
just build     # build all binaries
cargo test     # run integration tests
```

`integration_compositor.rs` additionally requires a running X11 or Wayland session.

## VM notes

The VM runs Fedora 41 Cloud Edition. It is headless (no GUI) and accessible via SSH on port 2222. The VM image is stored in `~/vms/lunaris-ebpf/` and is not committed to this repo.

If the VM disk gets corrupted or you want to start fresh:
```bash
rm ~/vms/lunaris-ebpf/fedora-ebpf.qcow2
qemu-img create -f qcow2 -F qcow2 \
  -b ~/vms/lunaris-ebpf/fedora-base.qcow2 \
  ~/vms/lunaris-ebpf/fedora-ebpf.qcow2 20G
```

## Part of

[Lunaris](https://github.com/lunaris-sys) — a Linux desktop OS built around a system-wide knowledge graph.
