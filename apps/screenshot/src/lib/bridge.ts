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

/// What came back when we asked for the screen.
///
/// THREE OUTCOMES, NOT TWO, and the split is the point. This used to return
/// `string | null`, so "there is no host to ask" and "a host is here and it
/// cannot capture your screen" arrived as the same `null` and the caller
/// answered both with the same invented desktop. A browser has no screen to
/// capture and a sample IS the answer there. A shipped app on a compositor
/// without screencopy has a screen it could not get, and drawing a fake one is
/// a picture of a machine that does not exist - which the person then annotates,
/// saves and sends, believing it came off their display.
export type Capture =
  /// The real screen, as a PNG data URL.
  | { kind: "image"; dataUrl: string }
  /// A host answered and cannot capture. Carries why, because "not supported by
  /// this compositor" and "the capture call failed" are different things to do
  /// something about.
  | { kind: "unavailable"; reason: string }
  /// No Tauri host at all: plain vite or the render harness.
  | { kind: "hostless" };

/// One display output the compositor offers.
export type Output = { index: number; name: string | null; width: number; height: number };

/// One toplevel window the compositor offers.
export type Window = { index: number; title: string | null; app_id: string | null };

/// What this machine can be asked to photograph.
///
/// Empty lists are an honest answer and not a failure: a compositor that
/// advertises no toplevels has none open, and the picker then offers the screen
/// alone rather than an empty list with no explanation.
export async function captureSources(): Promise<{ outputs: Output[]; windows: Window[] }> {
  if (!isTauri()) return { outputs: [], windows: [] };
  const [outputs, windows] = await Promise.all([
    invoke<Output[]>("list_outputs").catch(() => []),
    invoke<Window[]>("list_windows").catch(() => []),
  ]);
  return { outputs, windows };
}

/// Capture one window by the index `list_windows` gave it.
export async function captureWindow(index: number): Promise<Capture> {
  if (!isTauri()) return { kind: "hostless" };
  try {
    return { kind: "image", dataUrl: await invoke<string>("capture_window", { index, includeCursor: false }) };
  } catch (e) {
    return { kind: "unavailable", reason: String(e) };
  }
}

/// Capture one output by the index `list_outputs` gave it.
export async function captureOutput(index: number): Promise<Capture> {
  if (!isTauri()) return { kind: "hostless" };
  try {
    return { kind: "image", dataUrl: await invoke<string>("capture_output", { index, includeCursor: false }) };
  } catch (e) {
    return { kind: "unavailable", reason: String(e) };
  }
}

/// Capture the primary output.
export async function capturePrimary(): Promise<Capture> {
  if (!isTauri()) return { kind: "hostless" };
  try {
    if (!(await invoke<boolean>("capture_available"))) {
      return { kind: "unavailable", reason: "no screen capture on this compositor" };
    }
    return { kind: "image", dataUrl: await invoke<string>("capture_output", { index: 0, includeCursor: false }) };
  } catch (e) {
    return { kind: "unavailable", reason: String(e) };
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
