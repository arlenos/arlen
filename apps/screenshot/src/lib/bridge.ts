/// The Tauri command bridge for the screenshot app: the live capture / save / copy
/// destinations that back the annotate surface, over the coder's `src-tauri` commands
/// (which wrap `sdk/screen-capture` + `arlen-screenshot-core`). Every call is guarded
/// by `isTauri()` so the surface still renders + verifies under plain vite, where the
/// caller falls back to the synthetic fixture and the browser clipboard/download.

import { invoke } from "@tauri-apps/api/core";

/// Whether we run inside the Tauri host (so the native commands exist). Under vite
/// this is false and callers use their browser fallbacks.
export const isTauri = (): boolean =>
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/// Capture the primary output and return it as a PNG data URL for the annotate
/// canvas, or `null` when capture is unavailable (no host, or the compositor lacks
/// the screencopy interface) so the caller can fall back to the fixture.
export async function capturePrimary(): Promise<string | null> {
  if (!isTauri()) return null;
  try {
    if (!(await invoke<boolean>("capture_available"))) return null;
    return await invoke<string>("capture_output", { index: 0, includeCursor: false });
  } catch {
    return null;
  }
}

/// Save the flattened annotated PNG (base64, no data-url prefix) to the freedesktop
/// screenshots directory; resolves to the written path.
export async function saveScreenshot(pngBase64: string): Promise<string> {
  return invoke<string>("save_screenshot", { pngBase64 });
}

/// Copy the flattened annotated PNG (base64, no data-url prefix) to the system
/// clipboard as an image.
export async function copyPng(pngBase64: string): Promise<void> {
  await invoke("copy_png", { pngBase64 });
}

/// Surface a diagnostic on the app's stdout (the webview has no DevTools in the
/// Arlen shell). A no-op outside Tauri.
export function frontendLog(message: string): void {
  if (isTauri()) void invoke("frontend_log", { message }).catch(() => {});
}

/// The base64 body of a canvas PNG (drops the `data:image/png;base64,` prefix the
/// backend commands do not expect).
export function canvasPngBase64(c: HTMLCanvasElement): string {
  return c.toDataURL("image/png").split(",")[1] ?? "";
}
