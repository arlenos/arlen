<!--
  Ambient overlay — fixed-position pseudo-background div that
  pulses or tints in response to the focused app's
  shell.ambient state. Mounted once at the root layout level.
  pointer-events: none, z-index: 1 (above background, below
  the topbar at z=95 and any popover at z=100).

  See `docs/architecture/ambient-api.md`.
-->
<script lang="ts">
  import { focusedAmbient } from "$lib/stores/appStateStores";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";

  // Ambient is globally disable-able via `~/.config/arlen/
  // shell.toml [ambient] enabled = false`. Read once on mount and
  // re-read on external config writes (same event the toast config
  // follows); absent section means enabled.
  let ambientGloballyEnabled = $state(true);

  async function loadAmbientEnabled() {
    try {
      const cfg = await invoke<{ ambient?: { enabled?: boolean } }>(
        "get_shell_config",
      );
      ambientGloballyEnabled = cfg.ambient?.enabled ?? true;
    } catch {
      // Keep the default on error.
    }
  }

  $effect(() => {
    loadAmbientEnabled();
    let unlisten: (() => void) | null = null;
    listen("arlen://shell-config-changed", () => {
      loadAmbientEnabled();
    })
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => {});
    return () => {
      unlisten?.();
    };
  });

  // Animation duration mapping. The CSS `--ambient-speed-ms`
  // variable drives the keyframe iteration period.
  function durationMs(speed: string): number {
    switch (speed) {
      case "slow":
        return 3000;
      case "fast":
        return 800;
      case "medium":
      default:
        return 1500;
    }
  }
</script>

{#if ambientGloballyEnabled && $focusedAmbient}
  {@const a = $focusedAmbient}
  <div
    class="ambient-overlay"
    class:ambient-pulse={a.effect === "pulse"}
    class:ambient-tint={a.effect === "tint"}
    style="
      --ambient-color: var(--color-{a.color}, var(--color-accent));
      --ambient-intensity: {Math.max(0, Math.min(0.5, a.intensity))};
      --ambient-speed-ms: {durationMs(a.speed)}ms;
    "
  ></div>
{/if}

<style>
  .ambient-overlay {
    position: fixed;
    inset: 0;
    pointer-events: none;
    z-index: 1;
    /* Defence in depth: clamp at the use site even if a
       non-SDK producer pushed an out-of-range intensity. */
    --clamped-intensity: max(0, min(0.5, var(--ambient-intensity, 0)));
  }

  .ambient-pulse {
    background: var(--ambient-color);
    opacity: 0;
    animation: ambient-pulse var(--ambient-speed-ms) ease-in-out infinite;
  }
  @keyframes ambient-pulse {
    0%, 100% {
      opacity: 0;
    }
    50% {
      opacity: var(--clamped-intensity);
    }
  }

  .ambient-tint {
    background: var(--ambient-color);
    opacity: var(--clamped-intensity);
    transition: opacity var(--duration-medium, 200ms) var(--ease-out, ease-out);
  }
</style>
