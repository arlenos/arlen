<script lang="ts">
  /// Compact card for an artifact surfaced by a command, in the
  /// visual language of the harness artifact cards. The full
  /// renderer family graduates into the kit later; until then this
  /// card names the artifact without pretending to open it.
  import { FileText, Image as ImageIcon, Package } from "lucide-svelte";

  let {
    title,
    kind,
    summary = null,
  }: {
    title: string;
    kind: string;
    summary?: string | null;
  } = $props();

  const Icon = $derived(
    kind === "image" ? ImageIcon : kind === "document" ? FileText : Package,
  );
</script>

<div class="artifact-card">
  <span class="ac-icon"><Icon size={16} strokeWidth={1.5} /></span>
  <span class="ac-text">
    <span class="ac-title">{title}</span>
    {#if summary}
      <span class="ac-summary">{summary}</span>
    {/if}
  </span>
</div>

<style>
  .artifact-card {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .ac-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--height-control, 28px);
    height: var(--height-control, 28px);
    flex-shrink: 0;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--foreground) 7%, transparent);
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
  }

  .ac-text {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }
  .ac-title {
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--foreground);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ac-summary {
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
