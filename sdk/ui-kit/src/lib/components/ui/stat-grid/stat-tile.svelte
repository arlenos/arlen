<script lang="ts">
  /// A read-only metric tile: a small label over a value, with an optional
  /// status dot. For daemon liveness, DB/graph sizes, mount state, etc.
  let {
    label,
    value,
    status,
    id,
  }: {
    label: string;
    value: string;
    /// Optional status dot: ok (accent/green), warn (amber), off (muted).
    status?: "ok" | "warn" | "off";
    id?: string;
  } = $props();
</script>

<div class="stat-tile" {id}>
  <div class="stat-label">{label}</div>
  <div class="stat-value">
    {#if status}
      <span class="dot {status}"></span>
    {/if}
    <span class="stat-value-text">{value}</span>
  </div>
</div>

<style>
  .stat-tile {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    padding: 0.75rem;
    border-radius: var(--radius-card);
    border: 1px solid color-mix(in srgb, var(--foreground) 10%, transparent);
    /* Card surface + elevation per theme (the proven Group pattern). */
    background: var(--card, color-mix(in srgb, var(--foreground) 3%, transparent));
    box-shadow: var(--shadow-card, none);
    min-width: 0;
  }

  .stat-label {
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }

  .stat-value {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    font-size: var(--text-md);
    font-weight: 500;
    color: var(--foreground);
    min-width: 0;
  }

  .stat-value-text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dot {
    width: 8px;
    height: 8px;
    /* Status dots ride the roundness scale (radius-chip), not the
       categorical pill radius: round at the default and rounder themes,
       squaring off in lock-step as the geometry is dialled down. */
    border-radius: var(--radius-chip, 4px);
    flex-shrink: 0;
  }
  .dot.ok {
    background: var(--color-accent, #22c55e);
  }
  .dot.warn {
    background: #f59e0b;
  }
  .dot.off {
    background: color-mix(in srgb, var(--foreground) 30%, transparent);
  }
</style>
