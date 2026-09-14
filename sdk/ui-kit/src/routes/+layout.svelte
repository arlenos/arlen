<script lang="ts">
  import { onMount } from "svelte";
  import { loadTheme, applyTokens, DARK_TOKENS, type SurfaceTokens } from "$lib/theme";
  import "../app.css";
  import { listen } from "@tauri-apps/api/event";

  // The shipped defaults before first render, so a dev route never flashes
  // an unthemed surface and never draws one the system does not ship.
  applyTokens(DARK_TOKENS);

  onMount(() => {
    // onMount's cleanup must be a sync function, not a Promise, so the async
    // setup runs in an IIFE and the returned cleanup invokes the unlisten once
    // it resolves.
    let unlisten: (() => void) | undefined;
    void (async () => {
      // Load tokens from backend (reads theme.toml)
      try {
        await loadTheme();
      } catch {
        // No Tauri backend (e.g. browser dev mode), the defaults already stand
      }
      // Subscribe to live theme changes
      unlisten = await listen<SurfaceTokens>("arlen://theme-changed", ({ payload }) => {
        applyTokens(payload);
      });
    })();
    return () => unlisten?.();
  });
</script>

<slot />
