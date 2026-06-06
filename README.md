# Arlen

Arlen is a capability-based, event-driven Linux desktop, built from scratch on top of Debian. It is in early development (pre-alpha), so things move and break often.

The core ideas are three. A system-wide knowledge graph that the whole desktop reads and writes, treated as real infrastructure rather than a per-app database. Capability tokens for every cross-component access, so a component only reaches what it was granted. And a modern shell that replaces the usual Linux desktop pieces, meant to be usable by non-technical people, with no telemetry and no lock-in.

The full architecture is written up in the technical report under `foundation/` (also published with a DOI). Read it before making architectural decisions, it is the ground truth. Author: Tim Kicker, University of Innsbruck.

## Layout

This is a monorepo for the tightly-coupled first-party code. A few things live in their own repos where independence matters, the Wayland compositor (a cosmic-comp fork that tracks upstream) and the published foundation paper.

```
arlen/
  contracts/   shared wire crates: event, audit, notification, modulesd protos
  daemons/     background services
  ai/          the AI layer (agent, providers, core, classifier, sandbox)
  sdk/         os-sdk, module-sdk, permissions, theme, ui-kit, tauri plugins
  apps/        first-party apps (desktop shell, settings, files, terminal, ...)
  forage/      CLI package manager
  store-backend/  app store backend
  themes/      GTK theme generator
  distro/      VM setup and dev environment
  docs/        architecture specs (shared across the tree)
```

The daemons are the spine of the system:

- `event-bus` carries events between components over Unix sockets.
- `knowledge` is the knowledge graph (SQLite write side, a graph engine for queries) plus the project and timeline features.
- `kernel-layer` normalizes eBPF tracepoints into events.
- `notification-daemon` is the D-Bus notification server.
- `modulesd` hosts sandboxed extension modules.
- `installd` installs packages, modules, and Flatpaks.
- `audit-daemon` is the append-only audit ledger.
- `anomaly-detector` is an advisory watcher that informs, never blocks.
- `xdg-portal` is the xdg-desktop-portal backend.

## Building

Each crate or workspace builds with cargo. The knowledge daemon vendors a C++ graph engine that needs `<cstdint>` included on newer GCC, which the repo handles through `.cargo/config.toml`.

```
cargo check --manifest-path daemons/knowledge/Cargo.toml
cargo test  --manifest-path sdk/Cargo.toml
```

The frontend apps use Tauri 2 with SvelteKit and Svelte 5. They share the UI kit under `sdk/ui-kit` through the `@arlen/ui-kit` alias, so components are never copied between apps.

CI runs a per-crate check and test matrix, clippy and rustfmt, and svelte-check on the apps. The compositor is checked out and pinned separately when building a full image.

## Status

Pre-alpha. The daemons, the SDK, the shell, and the AI layer exist and build, and large parts are covered by tests, but this is not something to run as a daily driver yet. Expect breaking changes without migration shims.
