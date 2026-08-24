/// True when running inside a Tauri webview. The screenshot loop and a plain
/// browser run without the runtime, and the difference decides whether a sample
/// is the honest answer or an invention: with no host to ask, a fixture is what
/// there is to show; with a host that refused, a fixture is a claim about a
/// mailbox nobody read.
///
/// Same one-liner as calendar, clock and meetings. The mailbox store had it
/// hand-rolled inline, which was correct and which `check-fixture-on-failure`
/// could not see - it looks for this name, so a store spelling it out on its own
/// reads to the gate exactly like a store that never asked.
export const tauriAvailable =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
