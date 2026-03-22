<script lang="ts">
  import { onMount } from "svelte";
  import { loadTheme, PANDA_TOKENS, applyTokens } from "$lib/theme";
  import "../app.css";

  onMount(async () => {
    try {
      await loadTheme();
    } catch {
      // Not running inside Tauri (e.g. browser dev mode without backend).
      // Apply built-in Panda tokens as fallback.
      applyTokens(PANDA_TOKENS);
    }
  });
</script>

<slot />
