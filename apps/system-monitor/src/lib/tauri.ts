/// True when running inside a Tauri webview. The screenshot loop and
/// plain-browser dev run without the runtime; the store uses this to tell a
/// MOCK (no backend at all, keep the optimistic fixture behaviour) apart from a
/// REAL failure (the backend was there and refused), which must never be
/// swallowed - a Stop that silently fails would read as a process killed.
/// Same helper the Files app and Settings already use.
export const tauriAvailable =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
