/// Trigger a browser download of some text as a file. Works in the Tauri webview
/// (a Blob object URL + a temporary anchor click), so no backend save path is
/// needed. Kept out of `export.ts` so that stays pure and unit-testable.
export function downloadText(filename: string, text: string, mime = "text/plain"): void {
  const url = URL.createObjectURL(new Blob([text], { type: mime }));
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}
