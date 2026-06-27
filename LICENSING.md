# Licensing

Arlen is free software. The operating system is licensed **AGPL-3.0-only**; the
wire/contract crates are **Apache-2.0** so they can be linked by separate
services; the component library is **MIT** so an app outside this tree can build
on it; a few vendored or upstream-derived components keep their original GPL
license. Machine-readable per-file labels (REUSE / SPDX) make this map
enforceable in CI; this document makes it readable.

## Per-component license map

| Path | License | Why |
|---|---|---|
| `apps/**`, `daemons/**`, `ai/**`, `sdk/**` (except the rows below), `forage/**`, `themes/**` | `AGPL-3.0-only` | the OS itself: copyleft + anti-SaaS |
| `sdk/ui-kit/**` (the component library) | `MIT` | permissive so an app that is not part of the OS can use the same components |
| `contracts/**` (wire / proto crates) | `Apache-2.0` | permissive so a separate (possibly closed) service can link the wire types; carries a patent grant |
| `daemons/xdg-portal/**`, `sdk/tauri-plugin-*`, `sdk/arlen-input-client` | `GPL-3.0-only` | xdg-desktop-portal / cosmic-derived code, kept under its upstream license |
| `daemons/kernel-layer/**` | `GPL-2.0-only` | eBPF programs using kernel GPL-only helpers |

The compositor (a cosmic-comp fork, separate repository) is `GPL-3.0`, and the
bundled `pi` agent is `MIT`; both are upstream licenses kept as-is.

The MIT label covers the kit's own source. Its Rust harness in
`sdk/ui-kit/src-tauri` depends on `sdk/theme`, which stays `AGPL-3.0-only`, so a
binary built from that harness is AGPL as a whole; the Svelte components, the
tokens and the stylesheets have no such dependency.

## How it is enforced

- **`LICENSES/`** holds the verbatim text of every license used
  (`AGPL-3.0-only`, `Apache-2.0`, `GPL-3.0-only`, `GPL-2.0-only`, `MIT`), per the
  [REUSE](https://reuse.software) specification.
- **Per-file SPDX headers** (`SPDX-FileCopyrightText` + `SPDX-License-Identifier`)
  label each source file; files that cannot carry a header (binaries, generated
  output, data) are labeled in `REUSE.toml`. `reuse lint` checks this in CI.
- Each crate's `Cargo.toml` carries the matching `license` field.
- **`cargo-deny`** (`deny.toml`) gates the dependency tree against a license
  allowlist, and a generated `THIRD-PARTY-NOTICES` artifact records the licenses
  of bundled dependencies.

## Reusing code from this repository

Lifted third-party code keeps its **original** copyright and license, not
Arlen's: such a file carries its upstream SPDX header and is recorded in
`THIRD-PARTY-NOTICES`. Only AGPL-compatible code is accepted (MIT/BSD/ISC/Zlib/
Apache-2.0/GPL-3.0/LGPL/MPL-2.0/AGPL); GPL-2.0-only, proprietary, unknown and
CC-BY-SA sources are not.

The top-level `LICENSE` file (the full AGPL-3.0 text for the project as a whole)
is maintained separately.
