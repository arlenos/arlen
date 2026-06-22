# Testing capability matrix

What the coder can verify itself (headless, every change) versus what only Tim's
metal can confirm. Read this to know your verify surface instead of re-inferring it
from code inspection each time. Seeded from the test-capability audit; refines
`docs/architecture/test-infrastructure-plan.md` (the layer model) and
`docs/architecture/agent-self-testing-plan.md` (the improvement plan).

The iron rule still holds: compiling + unit tests passing is NOT "done" for anything
with a visible result. Confirm it through a test layer below, or report it as
metal-gated in `docs/architecture/coder-reports.md`. Never mark a visible-result
change done off a green build alone.

## Headless (verify here, every change)

- **Rust backend logic + IPC.** `cargo nextest run` per crate (`just test`, or
  `just test <crate>`). 482 test files, ~3869 `#[test]`/`#[tokio::test]` across ~47
  separate workspaces (there is no root workspace; the run is a per-crate loop).
  installd and knowledge are serial-pinned (`--test-threads=1`); they mutate shared
  process/filesystem state. knowledge needs `CXXFLAGS=-include cstdint` for the
  vendored lbug/Kuzu C++. Doctests run separately (`cargo test --doc`; nextest skips
  them).
- **Daemon IPC integration.** `dev/integration` (`EphemeralStack`): spins real
  daemons over a temp socket dir and drives the wire protocol end to end
  (`integration_backend_smoke`). This is the assembled-daemon path, still headless.
- **Frontend logic.** `vitest` for the apps that declare unit tests (harness, files,
  sdk/ui-kit; 140 tests). Gated in CI (the `frontend` job) and mirrored locally by
  `just check`. Catches store/transform/parse regressions, not render.
- **Static gates.** `cargo clippy -D warnings` (`just lint`; note the tree-wide
  `empty_line_after_doc_comments` debt makes this red on several pre-existing crates,
  independent of new work) and `svelte-check` (`just check`). CI clippy on the sdk/ai
  workspaces is advisory (`|| true`); the `-D warnings` gate is local-only.
- **Compositor render behavior.** The separate compositor repo has a headless
  render-readback harness with golden-PNG comparison (e.g. the window-header golden
  test). It covers element rasterisation and header geometry behaviorally without a
  GPU. `sdk/theme` has its own golden resolve test for generator output.
- **Frontend + full-app screenshots.** `dev/screenshot/shoot.py` loads a frontend URL
  in WebKitWebDriver (frontend isolated); `dev/screenshot/shoot_app.py` drives the
  real Tauri binary through `tauri-driver` under Xvfb (backend + webview together,
  IPC + render). Gated on `tauri-driver` + `Xvfb` + a built app binary being present.

## Metal-gated (report in coder-reports.md, never mark done)

- Full pixel fidelity, real-GPU rendering, fractional-scale visual correctness, the
  genuine "does it look right" judgement. The readback harness checks regions/hashes,
  not GPU-accurate output.
- Live compositor behavior on real hardware beyond the readback harness: window and
  input routing, the nested-compositor position/scale class, layer-shell surface
  state a headless webview cannot see. (WLCS would move much of this headless; not
  wired yet.)
- eBPF (`daemons/kernel-layer`): needs privileged load on a real or VM kernel.
- Audio playback (notification sound decode + PipeWire output): needs a live audio
  device. The sound-name resolution logic (`notification-daemon/src/sound.rs`) IS
  headless-tested; the decode + play path is metal.
- Live session-bus D-Bus delivery semantics where a real bus is required (directed
  signal visibility, bus-policy enforcement); the per-caller gate logic itself is
  unit-tested.

## Planned (would shrink the metal bottleneck; not built)

- **WLCS** in-process compositor behavior/protocol tests against the cosmic-comp fork
  (the `wlcs_server_integration` hooks). The highest-leverage un-gate for the
  "compiles but shows nothing" class.
- **VKMS** headless DRM backend + per-frame CRC golden-frame comparison. Needs
  `modprobe vkms` (a root action on Tim's machine, the VM, or CI).
- **cargo-mutants** mutation pass on the security-critical and pure-logic crates
  (capability `decide`, audit ledger, compensation, the gate, KG promotion/scoping).
  Mutation score is the real quality signal, not line coverage. Tool not installed.
- **sccache** + a binary cache for the ~90-120s knowledge cold-link and repeat
  integration runs. Tool not installed.

## How to write the tests (discipline)

- Oracle from INTENT, not observed output. Assert what the job/spec/docs say the code
  SHOULD do, never just what it currently returns (asserting your own just-written
  output pins your own bugs green).
- Before a test counts as protection, confirm it would FAIL on a plausible wrong
  implementation (a mini-mutant in your head).
- Prefer property-based tests (`proptest`/`quickcheck`) for pure logic (parsers,
  encoders, the capability decide, the graph predicates): assert invariants, not
  example outputs.
