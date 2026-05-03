<!--
  Ambient overlay — fixed-position pseudo-background div that
  pulses or tints in response to the focused app's
  shell.ambient state. Mounted once at the root layout level.
  pointer-events: none, z-index: 1 (above background, below
  TopBar at z=50 and any popover at z=100).

  See `docs/architecture/ambient-api.md`.
-->
<script lang="ts">
  import { focusedAmbient } from "$lib/stores/appStateStores";

  // Ambient is globally disable-able via `~/.config/lunaris/
  // shell.toml [ambient] enabled = false`. The shell config
  // store would expose this; for Phase 1 we read from a
  // simple Tauri command on mount and skip render when off.
  // TODO: wire shell.toml ambient.enabled — Phase 6.
  let ambientGloballyEnabled = $state(true);

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
    transition: opacity 0.3s ease-out;
  }
</style>
