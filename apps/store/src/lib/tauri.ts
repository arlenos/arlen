/// True when running inside a Tauri webview. The screenshot loop and a plain
/// browser run without the runtime, and the difference decides whether a sample
/// is the honest answer or an invention: with no host to ask, a fixture is what
/// there is to show; with a host that refused, a fixture is a catalogue nobody
/// read, and in a store that can cost money.
///
/// Same one-liner as mail, files and calendar; `check-fixture-on-failure` looks
/// for this name.
export const tauriAvailable =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
