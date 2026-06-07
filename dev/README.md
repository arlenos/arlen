# dev

Developer tooling for Arlen: build, test, and run the system locally. No
application code, this is the glue that turns the monorepo into a running stack.
The bootable image / distribution (mkosi, Fedora base, packaging) is separate
future work and gets its own `distro/` directory when it exists; until then the
`build-iso` recipe here is a stub.

## What's here

```
dev/
├── justfile              the one task entrypoint (run from anywhere)
├── process-compose.yaml  declarative full daemon-stack runner
├── scripts/              dev/install helpers (portal, modulesd, settings, ui-kit sync)
├── vm/                   eBPF kernel-layer QEMU VM (off-host, by design)
├── integration/          cross-daemon integration test crate (arlen-integration)
└── workspace-deps.toml   shared dependency version reference
```

## The three tiers

The dev loop is not one size. One `just` entrypoint, three tiers:

```bash
just                  # list recipes
just dev              # full daemon stack via process-compose (TUI)
just dev --profile shell    # + nested compositor + desktop-shell
just dev --profile portal   # + xdg-portal + settings (the old --with-portal)
just dev-ui harness   # quick: one Tauri app via cargo tauri dev (fast UI loop)
just vm               # eBPF kernel-layer in QEMU
```

`process-compose` models the real start order: `event-bus` creates its sockets,
`knowledge` waits on them (a readiness probe), the AI layer waits on knowledge,
and so on. It replaces the old hand-rolled tmux script with dependency-aware
startup, health checks, per-process logs, restart-on-crash, and a TUI. One-time
install: `just setup` (downloads the static binary into `~/.local/bin`, no sudo).
For scripted/detached use, `just dev -D` runs headless; reattach with
`process-compose attach`.

The eBPF component (kernel-layer) runs only in the VM: a kernel bug must not
destabilise the host.

## Build / test / verify

```bash
just check            # local mirror of CI: cargo check every crate + svelte-check every frontend
just build [crate]    # build one crate (e.g. just build daemons/knowledge) or all
just test [crate]     # test one crate or the whole matrix
just lint             # clippy -D warnings across all crates
just fmt              # cargo fmt across all crates
just integration      # the cross-daemon integration tests (arlen-integration)
```

`just check` is the fastest way to confirm a change the way CI will before
pushing. `CXXFLAGS=-include cstdint` is set for the knowledge/lbug build.

## VM notes

The VM runs Fedora Cloud, headless, SSH on port 2222, image under
`~/vms/arlen-ebpf/` (not committed). First-time setup:

```bash
sudo pacman -S qemu-system-x86 qemu-img cdrtools   # Arch/EndeavourOS
dev/vm/install-aya-toolchain.sh
dev/vm/setup-vm.sh setup && dev/vm/setup-vm.sh start
```

## Part of

[Arlen](https://github.com/arlenos): a Linux desktop OS built around a
system-wide knowledge graph.
