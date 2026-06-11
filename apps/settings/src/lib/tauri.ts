/// Whether the Tauri runtime is present. False when the frontend runs
/// standalone in a plain browser (vite dev, the screenshot loop); every
/// Tauri call outside a store's own error handling must check this, or a
/// synchronous throw in a `$effect`/`onMount` takes the whole route tree
/// down with it.
export const tauriAvailable: boolean =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
