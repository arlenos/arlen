/// True when running inside a Tauri webview. The shell always has the runtime in
/// production; plain-browser dev does not. Stores use this to tell a MOCK (no
/// backend at all, keep the optimistic fixture so the surface stays reviewable)
/// apart from a REAL failure (the daemon was there and refused), which must not
/// be swallowed. Same helper the Files app, Settings, system-monitor and the
/// text editor use.
export const tauriAvailable =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
