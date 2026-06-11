<script lang="ts">
  /// The Console archetype's block primitive (design-system.md §5.3,
  /// terminal.md §4.2): one command plus its result, rendered as a
  /// quiet unit. The command stays the star — the header row carries
  /// the marker, the command in mono and the result (exit chip only
  /// when something went wrong, duration always once finished);
  /// actions show on hover only; a thin divider closes the block.
  ///
  /// The host supplies the context line (the prompt: path + git),
  /// the origin marker, optional header-right content (e.g. a table
  /// lens toggle), hover actions, and the body. The body sits flush
  /// below the header; for plain text output it is a `GridRegion`.
  import type { Snippet } from "svelte";

  let {
    command,
    exitCode = null,
    durationMs = null,
    running = false,
    originLabel = null,
    context,
    marker,
    lens,
    actions,
    children,
  }: {
    /// The command line that ran. Arbitrary bytes by contract — it is
    /// rendered as text only.
    command: string;
    /// Exit code once finished; null while running. Zero renders no
    /// chip (the absence of an error is the status), non-zero renders
    /// an error-tinted chip.
    exitCode?: number | null;
    /// Wall-clock duration once finished; null while running.
    durationMs?: number | null;
    /// True while the command is still executing — shows the quiet
    /// running indicator instead of the result.
    running?: boolean;
    /// Optional origin word in the header meta cluster (e.g. "agent"
    /// on agent-issued blocks); null renders nothing, so the common
    /// you-ran path stays silent.
    originLabel?: string | null;
    /// The prompt context (path + git), rendered inline before the
    /// marker — one line like a real shell prompt.
    context?: Snippet;
    /// The origin marker rendered before the command (prompt char).
    marker?: Snippet;
    /// Optional header-right inline content (e.g. the table lens
    /// toggle) — visible always, sits before the result.
    lens?: Snippet;
    /// Hover-only actions at the right edge of the header.
    actions?: Snippet;
    /// The block body, flush below the header.
    children?: Snippet;
  } = $props();

  /// 12ms / 1.2s / 41.3s / 2m 05s — terse, tabular-safe.
  function formatDuration(ms: number): string {
    if (ms < 1000) return `${ms}ms`;
    const s = ms / 1000;
    if (s < 60) return `${s < 10 ? s.toFixed(1) : Math.round(s)}s`;
    const m = Math.floor(s / 60);
    const rest = Math.round(s % 60);
    return `${m}m ${String(rest).padStart(2, "0")}s`;
  }
</script>

<div class="console-block" class:failed={exitCode !== null && exitCode !== 0}>
  <div class="cb-header">
    {#if context}
      <span class="cb-context">{@render context()}</span>
    {/if}
    {#if marker}
      <span class="cb-marker">{@render marker()}</span>
    {/if}
    <span class="cb-command">{command}</span>
    <span class="cb-spacer"></span>
    {#if lens}
      <span class="cb-lens">{@render lens()}</span>
    {/if}
    {#if actions}
      <span class="cb-actions">{@render actions()}</span>
    {/if}
    {#if originLabel}
      <span class="cb-origin">{originLabel}</span>
    {/if}
    {#if running}
      <span class="cb-running" aria-label="Still running">
        <span class="cb-running-dot"></span>
        running
      </span>
    {:else}
      {#if exitCode !== null && exitCode !== 0}
        <span class="cb-exit">exit {exitCode}</span>
      {/if}
      {#if durationMs !== null}
        <span class="cb-duration">{formatDuration(durationMs)}</span>
      {/if}
    {/if}
  </div>

  {#if children}
    <div class="cb-body">{@render children()}</div>
  {/if}
</div>

<style>
  .console-block {
    display: flex;
    flex-direction: column;
    padding: 12px 0;
    border-bottom: 1px solid
      color-mix(in srgb, var(--foreground) 7%, transparent);
  }

  /* The prompt context sits inline before the command, one line like
     a real shell prompt; it yields (truncates) before the command
     ever has to. */
  .cb-context {
    flex-shrink: 1;
    min-width: 0;
    max-width: 45%;
    overflow: hidden;
  }

  .cb-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 16px;
    min-height: var(--height-control-compact, 24px);
  }

  .cb-marker {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 0.8125rem;
    line-height: 1.5;
  }

  .cb-command {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 0.8125rem;
    line-height: 1.5;
    color: var(--foreground);
    white-space: pre-wrap;
    word-break: break-word;
    min-width: 0;
  }

  .cb-spacer {
    flex: 1;
  }

  .cb-lens {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
  }

  /* Actions appear on block hover only — the resting block is quiet. */
  .cb-actions {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    opacity: 0;
    transition: opacity var(--duration-fast, 150ms) var(--ease-out, ease);
  }
  .console-block:hover .cb-actions,
  .cb-actions:focus-within {
    opacity: 1;
  }

  .cb-running {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .cb-running-dot {
    width: 6px;
    height: 6px;
    border-radius: var(--radius-full);
    background: var(--color-accent, var(--primary));
    animation: cb-running-pulse 1.6s ease-in-out infinite;
  }
  /* The pulse floor stays high enough that a still frame never
     reads the accent dot as gray. */
  @keyframes cb-running-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.65; }
  }

  .cb-origin {
    flex-shrink: 0;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

  /* Failure is marked by the word in error color, nothing more — the
     same rendering the sidebar history rows use. */
  .cb-exit {
    flex-shrink: 0;
    color: var(--color-error);
    font-size: 0.75rem;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
  }

  .cb-duration {
    flex-shrink: 0;
    font-size: 0.75rem;
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

  .cb-body {
    padding: 8px 16px 0;
  }
</style>
