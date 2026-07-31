<script lang="ts">
  /// The on-demand node-detail panel (knowledge-app.md §3.6, scaffold). KA-R1 shows
  /// the selected node's basics; the bounded-neighbourhood node view + deep
  /// provenance lineage are KA-R7.
  import { X } from "lucide-svelte";
  import type { FileEntry } from "@arlen/ui-kit/components/browser";
  import { formatModified } from "@arlen/ui-kit/components/browser";
  import { t } from "$lib/i18n/messages";

  let { entry, onclose }: { entry: FileEntry; onclose: () => void } = $props();
</script>

<aside class="kn-detail" aria-label={$t("k.detail.title")}>
  <header class="kn-detail-head">
    <span class="kn-detail-title">{$t("k.detail.title")}</span>
    <button type="button" class="kn-detail-close" onclick={onclose} aria-label={$t("k.close")}>
      <X size={15} strokeWidth={2} />
    </button>
  </header>

  <div class="kn-detail-name">{entry.name}</div>

  {#if entry.modified_unix != null}
    <div class="kn-kv">
      <span class="kn-k">{$t("k.detail.when")}</span>
      <span class="kn-v">{formatModified(entry.modified_unix)}</span>
    </div>
  {/if}

  <p class="kn-detail-more">{$t("k.detail.more")}</p>
</aside>

<style>
  .kn-detail {
    flex: 0 0 17rem;
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    padding: 0.75rem 0.9rem;
    border-inline-start: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    overflow-y: auto;
  }
  .kn-detail-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .kn-detail-title {
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-primary) 42%, transparent);
  }
  .kn-detail-close {
    display: inline-flex;
    padding: 0.2rem;
    border: none;
    border-radius: var(--radius-input);
    background: transparent;
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
    cursor: pointer;
  }
  .kn-detail-close:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    color: var(--color-fg-primary);
  }
  .kn-detail-name {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--color-fg-primary);
    line-height: 1.4;
  }
  .kn-kv {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
  }
  .kn-k {
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .kn-v {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 80%, transparent);
  }
  .kn-detail-more {
    margin: 0.4rem 0 0;
    font-size: var(--text-2xs);
    line-height: 1.45;
    color: color-mix(in srgb, var(--color-fg-primary) 48%, transparent);
  }
</style>
