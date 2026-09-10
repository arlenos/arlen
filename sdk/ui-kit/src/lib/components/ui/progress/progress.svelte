<script lang="ts">
  /// A read-only progress bar: a filled track showing a 0..100 value, for
  /// determinate work like a download or an import. Not interactive.
  let {
    value,
    id,
    label,
    labelledby,
  }: {
    /// Progress from 0 to 100. Clamped.
    value: number;
    id?: string;
    /// What this bar is the progress OF, for a reader who cannot see what it
    /// sits under. A `role="progressbar"` with no accessible name announces a
    /// percentage and nothing else - four of them in one jobs list are four
    /// numbers with no subjects. Axe reports it as `aria-progressbar-name`
    /// (serious); measured on 11 September on the shell's jobs zone.
    label?: string;
    /// The id of a visible element that already names it - preferred over
    /// `label`, since a name the reader can also SEE is one fewer string to
    /// keep in step.
    labelledby?: string;
  } = $props();

  const pct = $derived(Math.max(0, Math.min(100, value)));
</script>

<div
  class="progress"
  {id}
  role="progressbar"
  aria-label={labelledby ? undefined : label}
  aria-labelledby={labelledby}
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
