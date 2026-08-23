/// Name the window in the reader's language.
///
/// `<svelte:head><title>` sets the document title, which never leaves the
/// webview. The name the topbar's menu bar and the workspace overview show is
/// the native window title, which came from `tauri.conf.json` and so stayed
/// "Files" on a German machine while the catalog had "Dateien" all along.
///
/// Best-effort: outside Tauri (a plain vite session, a test) there is no window
/// to name, and an app that keeps its configured title is a working app.
export async function setWindowTitle(title: string): Promise<void> {
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    await getCurrentWindow().setTitle(title);
  } catch (e) {
    console.warn("setWindowTitle: the window kept its configured name:", e);
  }
}
