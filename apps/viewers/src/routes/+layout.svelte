<script lang="ts">
  /// Root layout. The viewer has no persistent chrome of its own - each face
  /// (image / video frame / audio player) fills the window and draws its own
  /// auto-hide controls. The layout only loads the theme + suppresses the
  /// webview's native context menu so the app's own menus are the only ones.
  import "../app.css";
  import { onMount } from "svelte";
  import { initArlenTheme } from "@arlen/ui-kit/theme";
  import { initArlenLocale } from "@arlen/ui-kit/i18n";

  let { children } = $props();

  function suppressBrowserContextMenu(e: MouseEvent): void {
    if ((e.target as HTMLElement | null)?.closest?.("[data-allow-browser-context]")) return;
    e.preventDefault();
  }

  // The topbar and the workspace overview show the NATIVE window title,
  // not the document one below, so it has to be set - and set again when
  // the language changes, which is why this reads `$t` instead of firing
  // once at startup.
  $effect(() => {
    void setWindowTitle($t("v.app.title"));
  });

  onMount(() => {
    void initArlenTheme();
    void initArlenLocale();
    document.addEventListener("contextmenu", suppressBrowserContextMenu);
    return () => document.removeEventListener("contextmenu", suppressBrowserContextMenu);
  });
  import { t } from "$lib/i18n/messages";
  import { setWindowTitle } from "$lib/window-title";
</script>

<svelte:head>
  <!-- The document title: what a screen reader announces for the window
       and what a task switcher shows. Every Arlen app was missing one,
       which axe reports as `document-title` on every surface. -->
  <title>{$t("v.app.title")}</title>
</svelte:head>

{@render children?.()}
