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
/// Why a capture did not happen.
///
/// `no-screencopy` is a property of the compositor and stays true until it gains
/// the interface; `refused` is one call failing and may not repeat. A person can
/// act on the first and only retry the second.
export type CaptureRefusal = "no-screencopy" | "refused";

export type Capture =
  /// The real screen, as a PNG data URL.
  | { kind: "image"; dataUrl: string }
  /// A host answered and cannot capture. Carries WHY as a word, because "not
  /// supported by this compositor" and "the capture call failed" are different
  /// things to do something about - and because the page draws this, so a
  /// sentence here is a sentence in English on every screen. The compositor's own
  /// words go to the log instead.
  | { kind: "unavailable"; why: CaptureRefusal }
  /// No Tauri host at all: plain vite or the render harness.
  | { kind: "hostless" };

/// One display output the compositor offers.
export type Output = { index: number; name: string | null; width: number; height: number };

/// One toplevel window the compositor offers.
export type Window = { index: number; title: string | null; app_id: string | null; identifier: string | null };

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

/// Capture one window.
///
/// The identifier goes with the index because the list and the capture are two
/// separate Wayland connections: an index can point at a different window by
/// the time it is used, and the identifier cannot.
export async function captureWindow(index: number, identifier: string | null = null): Promise<Capture> {
  if (!isTauri()) return { kind: "hostless" };
  try {
    return { kind: "image", dataUrl: await invoke<string>("capture_window", { index, identifier, includeCursor: false }) };
  } catch (e) {
    console.warn("screenshot: capture_window refused", e);
    return { kind: "unavailable", why: "refused" };
  }
}

/// Capture one output.
///
/// The connector name goes with the index for the same reason the window's
/// identifier does: the list and the capture are two Wayland connections.
export async function captureOutput(index: number, name: string | null = null): Promise<Capture> {
  if (!isTauri()) return { kind: "hostless" };
  try {
    return { kind: "image", dataUrl: await invoke<string>("capture_output", { index, name, includeCursor: false }) };
  } catch (e) {
    console.warn("screenshot: capture_output refused", e);
    return { kind: "unavailable", why: "refused" };
  }
}

/// Capture the primary output.
export async function capturePrimary(): Promise<Capture> {
  if (!isTauri()) return { kind: "hostless" };
  try {
    if (!(await invoke<boolean>("capture_available"))) {
      return { kind: "unavailable", why: "no-screencopy" };
    }
    return { kind: "image", dataUrl: await invoke<string>("capture_output", { index: 0, includeCursor: false }) };
  } catch (e) {
    console.warn("screenshot: capture_output refused", e);
    return { kind: "unavailable", why: "refused" };
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

/// Close this window.
///
/// The app takes a picture, saves it and is done; before this it had no way to
/// go. `decorations: false` and no titlebar of its own meant the last state -
/// one sentence saying where the file went - sat on screen with no button to
/// press and no key that closed it, on a desktop where every other window has a
/// close control. Photographed on a German boot, 23 August.
///
/// Best-effort and guarded like every other call here: under plain vite there is
/// no window to close, and a tab that refuses to go away is the browser's own
/// rule rather than a fault of this app.
export async function closeWindow(): Promise<void> {
  if (!isTauri()) return;
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    await getCurrentWindow().close();
  } catch (e) {
    console.warn("closeWindow: the window stayed open:", e);
  }
}
