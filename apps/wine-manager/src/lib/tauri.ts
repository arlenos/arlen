/// True when running inside a Tauri webview. The screenshot loop and
/// plain-browser dev run without the runtime; guards keep window
/// controls and drag from throwing there (same pattern as the reader).
export const tauriAvailable =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
