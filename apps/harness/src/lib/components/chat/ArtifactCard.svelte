<script lang="ts">
  /// The chat reference for a pane artifact: a minimal one-line card (kind glyph,
  /// title, kind badge, open arrow) - NO peek/preview. Clicking opens the full
  /// render in the right pane. Used only for large text/data artifacts; small +
  /// visual artifacts render full inline instead.
  import { Code, FileText, Table, Workflow, ArrowUpRight } from "@lucide/svelte";
  import { kindLabel, type Artifact, type ArtifactKind } from "$lib/components/artifact/types";
  import { openPane } from "$lib/stores/artifact";

  let { artifact }: { artifact: Artifact } = $props();

  const glyph = (k: ArtifactKind) => {
    switch (k) {
      case "code":
        return Code;
      case "table":
        return Table;
      case "diagram":
        return Workflow;
      default:
        return FileText;
    }
  };
  const Glyph = $derived(glyph(artifact.kind));
  const title = $derived(artifact.meta.title ?? kindLabel(artifact.kind));
</script>

<button class="art-card" onclick={() => openPane(artifact)}>
  <span class="art-glyph"><Glyph size={16} strokeWidth={1.75} /></span>
  <span class="art-title">{title}</span>
  <span class="art-badge">{kindLabel(artifact.kind)}</span>
  <ArrowUpRight class="art-open" size={15} strokeWidth={2} />
</button>

<style>
  .art-card {
    display: flex;
    align-items: center;
    gap: 0.625rem;
    width: 100%;
    padding: 0.5rem 0.75rem;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-input);
    background: transparent;
    text-align: left;
    transition:
      background-color var(--duration-fast) var(--ease-out),
      border-color var(--duration-fast) var(--ease-out);
  }
  .art-card:hover {
    background: color-mix(in srgb, var(--foreground) 4%, transparent);
    border-color: color-mix(in srgb, var(--foreground) 18%, transparent);
  }
  .art-glyph {
    flex-shrink: 0;
    display: inline-flex;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  .art-title {
    flex: 1;
    min-width: 0;
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--foreground);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .art-badge {
    flex-shrink: 0;
    padding: 0.0625rem 0.375rem;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  :global(.art-open) {
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
  }
  .art-card:hover :global(.art-open) {
    color: var(--foreground);
  }
</style>
