<script lang="ts">
  /// A read-only progress bar: a filled track showing a 0..100 value, for
  /// determinate work like a download or an import. Not interactive.
  let {
    value,
    id,
  }: {
    /// Progress from 0 to 100. Clamped.
    value: number;
    id?: string;
  } = $props();

  const pct = $derived(Math.max(0, Math.min(100, value)));
</script>

<div
  class="progress"
  {id}
  role="progressbar"
  aria-valuenow={Math.round(pct)}
  aria-valuemin={0}
  aria-valuemax={100}
>
  <div class="progress-fill" style={`width:${pct}%`}></div>
</div>

<style>
  .progress {
    width: 100%;
    height: 6px;
    border-radius: var(--radius-full, 9999px);
    background: color-mix(in srgb, var(--foreground) 12%, transparent);
    overflow: hidden;
  }
  .progress-fill {
    height: 100%;
    border-radius: var(--radius-full, 9999px);
    background: var(--color-accent, var(--foreground));
    transition: width var(--duration-fast, 150ms) var(--ease-out, ease);
  }
</style>
