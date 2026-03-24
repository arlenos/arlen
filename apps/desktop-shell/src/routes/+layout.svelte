<script lang="ts">
  import { onMount } from "svelte";
  import { loadTheme, applyTokens, PANDA_TOKENS, type SurfaceTokens } from "$lib/theme";
  import "../app.css";
  import { initWindowListeners } from "$lib/stores/windows";
  import { listen } from "@tauri-apps/api/event";

  // Apply Panda tokens immediately before first render
  applyTokens(PANDA_TOKENS);

  onMount(async () => {
    initWindowListeners();
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
