<script lang="ts">
  import { onMount } from "svelte";
  import { initTheme } from "$lib/theme";
  import { activePopover, closePopover } from "$lib/stores/activePopover.js";
  import "../app.css";

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && $activePopover !== null) {
      e.preventDefault();
      closePopover();
    }
  }
  import { initWindowListeners } from "$lib/stores/windows";
  import { initContextMenuListeners } from "$lib/stores/contextMenu.js";
  import { initNotifications } from "$lib/stores/notifications.js";
  import { initWorkspaceListeners } from "$lib/stores/workspaces.js";
  import { initMenuListeners } from "$lib/stores/menus.js";
  import { initTabBarListeners } from "$lib/stores/tabBars";
  import { initIndicatorListeners } from "$lib/stores/indicators";
  import { initZoomListeners } from "$lib/stores/zoom";
  import { initWindowHeaderListeners } from "$lib/stores/windowHeaders";
  import { initProjects } from "$lib/stores/projects.js";
  import ContextMenu from "$lib/components/ContextMenu.svelte";
  import TabBar from "$lib/components/TabBar.svelte";
  import Indicator from "$lib/components/Indicator.svelte";
  import ZoomToolbar from "$lib/components/ZoomToolbar.svelte";
  import WindowHeader from "$lib/components/WindowHeader.svelte";
  import { Toaster } from "svelte-sonner";

  onMount(() => {
    initWindowListeners();
    initContextMenuListeners();
    initNotifications();
    initWorkspaceListeners();
    initMenuListeners();
    initTabBarListeners();
    initIndicatorListeners();
    initZoomListeners();
    initWindowHeaderListeners();
    initProjects();

    // Initialize theme system (loads appearance.toml, injects CSS vars,
    // subscribes to live theme-changed events from Rust).
    initTheme().catch(() => {});
  });
</script>

<svelte:window onkeydown={handleKeydown} />

<slot />
<ContextMenu />
<TabBar />
<Indicator />
<ZoomToolbar />
<WindowHeader />
<Toaster
  position="top-right"
  richColors
  expand={false}
  closeButton
  theme="dark"
  offset={44}
/>
