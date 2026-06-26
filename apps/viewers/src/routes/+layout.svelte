<script lang="ts">
  /// Root layout. The viewer has no persistent chrome of its own - each face
  /// (image / video frame / audio player) fills the window and draws its own
  /// auto-hide controls. The layout only loads the theme + suppresses the
  /// webview's native context menu so the app's own menus are the only ones.
  import "../app.css";
  import { onMount } from "svelte";
  import { initArlenTheme } from "@arlen/ui-kit/theme";

  let { children } = $props();

  function suppressBrowserContextMenu(e: MouseEvent): void {
    if ((e.target as HTMLElement | null)?.closest?.("[data-allow-browser-context]")) return;
    e.preventDefault();
  }

  onMount(() => {
    void initArlenTheme();
    document.addEventListener("contextmenu", suppressBrowserContextMenu);
    return () => document.removeEventListener("contextmenu", suppressBrowserContextMenu);
  });
</script>

{@render children?.()}
