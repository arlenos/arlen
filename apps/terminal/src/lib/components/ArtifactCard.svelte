<script lang="ts">
  /// An artifact surfaced by a command, named in plain text: title
  /// line plus a dimmed summary, no icon tile — the stream speaks
  /// text, not pictograms. The full renderer family graduates into
  /// the kit later; until then this names the artifact without
  /// pretending to open it.
  let {
    title,
    kind,
    summary = null,
  }: {
    title: string;
    kind: string;
    summary?: string | null;
  } = $props();
</script>

<div class="artifact-card">
  <span class="ac-title">{title}</span>
  <span class="ac-summary">
    {kind}{summary ? `, ${summary}` : ""}
  </span>
</div>

<style>
  .artifact-card {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .ac-title {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--foreground);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ac-summary {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
