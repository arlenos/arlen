<script lang="ts">
  let { signal = 0 }: { signal: number } = $props();
  // Zero (or unknown) signal shows zero bars — forcing a minimum of
  // one bar made "no signal" indistinguishable from a weak one.
  const bars = $derived(signal <= 0 ? 0 : Math.ceil(signal / 20));
</script>

<div class="signal-bars" role="img" aria-label="Signal strength {signal}%">
  {#each [1, 2, 3, 4, 5] as bar}
    <div class="signal-bar" class:active={bar <= bars} style:height="{bar * 3}px"></div>
  {/each}
</div>

<style>
  .signal-bars { display: flex; align-items: flex-end; gap: 1px; height: 14px; }
  .signal-bar { width: 3px; background: color-mix(in srgb, var(--color-fg-shell) 20%, transparent); border-radius: var(--radius-chip); }
  .signal-bar.active { background: var(--color-fg-shell); }
</style>
