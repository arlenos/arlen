<script lang="ts">
  /// A single row inside a Group card. Label on the left, control on
  /// the right, with an optional inline preview between them.
  import type { Snippet } from "svelte";

  let {
    label,
    id: rowId,
    description,
    control,
    preview,
  }: {
    label: string;
    /// Optional anchor id for deep-link scroll-to-setting.
    id?: string;
    description?: string;
    control?: Snippet;
    preview?: Snippet;
  } = $props();
</script>

<div class="row" id={rowId}>
  <div class="label">
    <div class="label-title">{label}</div>
    {#if description}
      <div class="label-desc">{description}</div>
    {/if}
  </div>
  {#if preview}
    <div class="preview">
      {@render preview()}
    </div>
  {/if}
  <div class="control">
    {@render control?.()}
  </div>
</div>

<style>
  .row {
    display: flex;
    align-items: center;
    gap: 0.875rem;
    padding: 0.75rem 1rem;
    min-height: 40px;
  }

  .label {
    flex: 1;
    min-width: 0;
  }

  .label-title {
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--foreground);
    line-height: 1.3;
  }

  .label-desc {
    font-size: 0.6875rem;
    line-height: 1.3;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    margin-top: 0.0625rem;
  }

  .preview,
  .control {
    flex-shrink: 0;
  }
</style>
