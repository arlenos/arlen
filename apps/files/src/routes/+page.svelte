<script lang="ts">
  /// The browser surface. The shared FileBrowser lands here with the
  /// kit browser-core increment; until then the page states the truth.
</script>

<div class="boot">
  <span class="boot-title">Files</span>
  <span class="boot-hint">The file browser is being built.</span>
</div>

<style>
  .boot {
    margin: auto;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
  }
  .boot-title {
    font-size: 0.75rem;
    font-weight: 500;
    color: var(--foreground);
  }
  .boot-hint {
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
</style>
