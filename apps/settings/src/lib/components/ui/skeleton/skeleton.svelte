<script lang="ts">
  /// Skeleton placeholder.
  ///
  /// Pulse-animated rounded rectangle that occupies the same
  /// shape as eventual content. Used by the QuickSettings layout
  /// editor to stand in for live tile data (slider tracks, chart
  /// regions, etc.) so the preview communicates "this is layout,
  /// not a state snapshot" without flickering as background-data
  /// arrives.
  ///
  /// Theme-aware via shell tokens: bg follows `--color-fg-shell`
  /// at low opacity, so it reads correctly in dark / light /
  /// monochrome themes without per-mode CSS.
  import type { HTMLAttributes } from "svelte/elements";
  import { cn } from "$lib/utils";

  type Props = HTMLAttributes<HTMLDivElement> & {
    class?: string;
  };

  let { class: className, ...rest }: Props = $props();
</script>

<div
  data-slot="skeleton"
  class={cn("arlen-skeleton", className)}
  {...rest}
></div>

<style>
  .arlen-skeleton {
    background: color-mix(in srgb, var(--color-fg-shell) 12%, transparent);
    border-radius: var(--radius-chip);
    animation: arlen-skeleton-pulse 1.8s ease-in-out infinite;
  }
  @keyframes arlen-skeleton-pulse {
    0%, 100% { opacity: 0.45; }
    50% { opacity: 0.85; }
  }
</style>
