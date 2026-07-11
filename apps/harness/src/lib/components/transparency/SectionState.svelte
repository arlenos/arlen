<script lang="ts">
  /// One honest, non-happy-path state for a transparency section: a quiet
  /// message, optionally tagged. The tag is the load-bearing honesty
  /// device: "Not measured yet" reads differently from "Nothing", and an
  /// unmeasured zero must never look like a measured none. Rendering only.
  let {
    tag = null,
    tone = "muted",
    message,
    hint = null,
  }: {
    /// A small pill (e.g. "Not measured yet", "AI is off"); omit for a
    /// plain message.
    tag?: string | null;
    /// The pill's tone. `muted` is the default neutral; `info` for a
    /// deliberate not-yet state; `off` for the AI-off state.
    tone?: "muted" | "info" | "off";
    /// The one-sentence state in plain language.
    message: string;
    /// An optional second line with more context.
    hint?: string | null;
  } = $props();
</script>

<div class="state">
  {#if tag}
    <span class="tag" data-tone={tone}>{tag}</span>
  {/if}
  <p class="msg">{message}</p>
  {#if hint}
    <p class="hint">{hint}</p>
  {/if}
</div>

<style>
  .state {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 0.375rem;
    padding: 0.625rem var(--space-row, 0.75rem) 0.875rem;
  }
  .tag {
    display: inline-flex;
    align-items: center;
    height: var(--height-tag, 20px);
    padding: 0 0.5rem;
    border-radius: var(--radius-chip);
    font-size: var(--text-2xs);
    font-weight: 500;
    letter-spacing: 0.02em;
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  .tag[data-tone="info"] {
    color: var(--color-accent);
    background: color-mix(in srgb, var(--color-accent) 14%, transparent);
  }
  .tag[data-tone="off"] {
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
    background: color-mix(in srgb, var(--foreground) 6%, transparent);
  }
  .msg {
    margin: 0;
    font-size: var(--text-sm);
    line-height: 1.5;
    color: color-mix(in srgb, var(--foreground) 75%, transparent);
  }
  .hint {
    margin: 0;
    font-size: var(--text-xs);
    line-height: 1.5;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    max-width: 60ch;
  }
</style>
