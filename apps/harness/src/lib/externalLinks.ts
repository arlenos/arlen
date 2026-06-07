import { invoke } from "@tauri-apps/api/core";

/// Svelte action for rendered-markdown containers: open any link the user
/// clicks in their browser/mail client (via the `open_url` backend command)
/// instead of letting the click navigate the Tauri webview, which would
/// replace the single-page app with the target site. Delegated on the
/// container, so it covers every link in the (re-rendered) content.
export function externalLinks(node: HTMLElement) {
  function onClick(event: MouseEvent) {
    const anchor = (event.target as HTMLElement | null)?.closest("a");
    const href = anchor?.getAttribute("href");
    if (!href) return;
    event.preventDefault();
    // The backend re-validates the scheme; a rejected URL just does nothing.
    invoke("open_url", { url: href }).catch((err) =>
      console.error("open_url failed", err),
    );
  }
  node.addEventListener("click", onClick);
  return {
    destroy() {
      node.removeEventListener("click", onClick);
    },
  };
}
