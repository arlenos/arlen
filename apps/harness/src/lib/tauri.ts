/// True when running inside a Tauri webview. Plain-browser dev has no runtime;
/// stores use this to tell a MOCK (no backend at all, keep the optimistic
/// fixture so the flow stays reviewable) apart from a REAL failure (the daemon
/// was there and refused), which must not be reported as success. Same helper
/// the Files app, Settings, system-monitor, the text editor and the shell use.
export const tauriAvailable =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
