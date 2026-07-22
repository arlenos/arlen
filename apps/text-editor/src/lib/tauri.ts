/// True when running inside a Tauri webview. The screenshot loop and
/// plain-browser dev run without the runtime; the AI-edit store uses this to
/// tell a MOCK (no backend at all, keep the optimistic fixture) apart from a
/// REAL failure (the backend was there and refused), which must never be
/// swallowed - a silently failed undo would claim an assistant edit was
/// reverted while it is still in the file. Same helper the Files app,
/// Settings and system-monitor use.
export const tauriAvailable =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
