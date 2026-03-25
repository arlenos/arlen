<script lang="ts">
  import { onMount } from "svelte";
  import { loadTheme, applyTokens, PANDA_TOKENS, type SurfaceTokens } from "$lib/theme";
  import "../app.css";
  import { initWindowListeners } from "$lib/stores/windows";
  import { initContextMenuListeners } from "$lib/stores/contextMenu.js";
  import { initNotificationListener } from "$lib/stores/notifications.js";
  import ContextMenu from "$lib/components/ContextMenu.svelte";
  import { Toaster } from "svelte-sonner";
  import { listen } from "@tauri-apps/api/event";

  // Apply Panda tokens immediately before first render
  applyTokens(PANDA_TOKENS);

  onMount(async () => {
    initWindowListeners();
    initContextMenuListeners();
    initNotificationListener();
    // Load tokens from backend (reads theme.toml)
    try {
      await loadTheme();
    } catch {
      // No Tauri backend (e.g. browser dev mode), Panda already applied
    }

    // Subscribe to live theme changes
    const unlisten = await listen<SurfaceTokens>("lunaris://theme-changed", ({ payload }) => {
      applyTokens(payload);
    });

    return unlisten;
  });
</script>

<slot />
<ContextMenu />
<Toaster
  position="top-right"
  richColors
  expand={false}
  closeButton
  theme="dark"
  offset={44}
/>
