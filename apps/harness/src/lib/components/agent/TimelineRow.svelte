<script lang="ts">
  /// One ledger-style row on the shared grid: fixed badge column |
  /// flexible body (subject line + detail line) | right-aligned time.
  /// The activity timeline, the behaviour list, and the notices all render
  /// through this, so every row in the dashboard rasters identically.
  import type { Tone } from "$lib/ledger";

  interface Annotation {
    text: string;
    tone?: Tone;
  }

  let {
    label,
    tone = "neutral",
    subject,
    subjectMeta = [],
    detail = [],
    time,
  }: {
    /// Badge text (the row's kind / state).
    label: string;
    /// Badge tone, mapped to theme tokens.
    tone?: Tone;
    /// The row's main line; truncates with the full text in the tooltip.
    subject: string;
    /// Small inline annotations after the subject (outcome, kind, …).
    subjectMeta?: Annotation[];
    /// Secondary line, parts joined with separators; wraps when long.
    detail?: Annotation[];
    /// Right-aligned relative time; omit for time-less rows.
    time?: string;
  } = $props();
</script>

<li class="row">
  <span class="badge" data-tone={tone}>{label}</span>
  <div class="body">
    <div class="line">
      <span class="subject" title={subject}>{subject}</span>
      {#each subjectMeta as m (m.text)}
        <span class="meta" data-tone={m.tone ?? "neutral"}>{m.text}</span>
      {/each}
    </div>
    {#if detail.length > 0}
      <div class="detail">
        {#each detail as d, i (i)}
          {#if i > 0}<span class="sep">·</span>{/if}
          <span class="detail-item" data-tone={d.tone ?? "neutral"}>{d.text}</span>
        {/each}
      </div>
    {/if}
  </div>
  {#if time !== undefined}
    <time class="time">{time}</time>
  {/if}
</li>

<style>
  .row {
    display: grid;
    grid-template-columns: 6rem minmax(0, 1fr) auto;
    column-gap: var(--space-row, 0.75rem);
    align-items: start;
    padding: 0.625rem var(--space-row, 0.75rem);
  }
  .badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    height: var(--height-tag, 20px);
    padding: 0 0.5rem;
    border-radius: var(--radius-chip);
    font-size: 0.6875rem;
    font-weight: 500;
    color: color-mix(in srgb, var(--foreground) 75%, transparent);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  .badge[data-tone="ok"] {
    color: var(--color-success);
    background: color-mix(in srgb, var(--color-success) 14%, transparent);
  }
  .badge[data-tone="warn"] {
    color: var(--color-error);
    background: color-mix(in srgb, var(--color-error) 14%, transparent);
  }
  .badge[data-tone="info"] {
    color: var(--color-accent);
    background: color-mix(in srgb, var(--color-accent) 14%, transparent);
  }
  .body {
    min-width: 0;
  }
  /* The first body line shares the badge's height as its line box, so the
     badge, the subject, and the time sit on one optical line. */
  .line {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    line-height: var(--height-tag, 20px);
  }
  .subject {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 0.8125rem;
    color: var(--foreground);
  }
  .meta {
    flex-shrink: 0;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .meta[data-tone="warn"] {
    color: var(--color-error);
  }
  .detail {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0.25rem;
    margin-top: 0.125rem;
    font-size: 0.75rem;
    line-height: 1.4;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .detail-item[data-tone="warn"] {
    color: var(--color-error);
  }
  .sep {
    opacity: 0.5;
  }
  .time {
    font-size: 0.75rem;
    line-height: var(--height-tag, 20px);
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    white-space: nowrap;
  }
</style>
