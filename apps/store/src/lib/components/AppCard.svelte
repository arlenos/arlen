<script lang="ts">
  /// The one app card, reused everywhere (collections, results, the all-apps
  /// grid): icon tile, name, one-line summary. Trust is the only chip and only
  /// when it has something to say - curated stays silent, community is flagged
  /// (§3); the package format never shows in browse. A bridge or module names
  /// its kind, because installing one does something different.
  import { Badge } from "@arlen/ui-kit/components/ui/badge";
  import { t } from "$lib/i18n/messages";
  import { trustOf, type StoreCard } from "$lib/stores/catalog";
  import IconTile from "./IconTile.svelte";

  let { app, onopen }: { app: StoreCard; onopen: (id: string) => void } = $props();
</script>

<button type="button" class="card" id={`app-${app.id}`} onclick={() => onopen(app.id)}>
  <IconTile icon={app.icon} name={app.name} />
  <span class="body">
    <span class="name">{app.name}</span>
    <span class="summary">{app.summary}</span>
    {#if trustOf(app) === "community" || app.kind !== "app"}
      <span class="chips">
        {#if trustOf(app) === "community"}
          <Badge variant="outline">{$t("st.trust.community")}</Badge>
        {/if}
        {#if app.kind === "bridge"}
          <Badge variant="outline">{$t("st.kind.bridge")}</Badge>
        {:else if app.kind === "module"}
          <Badge variant="outline">{$t("st.kind.module")}</Badge>
        {/if}
      </span>
    {/if}
  </span>
</button>

<style>
  .card {
    display: flex;
    gap: 0.75rem;
    align-items: flex-start;
    padding: 0.75rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    border-radius: var(--radius-card);
    background: color-mix(in srgb, var(--color-fg-primary) 2%, transparent);
    text-align: start;
    transition: background var(--duration-fast, 150ms) ease, border-color var(--duration-fast, 150ms) ease;
  }
  .card:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 5%, transparent);
    border-color: color-mix(in srgb, var(--color-fg-primary) 16%, transparent);
  }
  .card:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: 2px;
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    min-width: 0;
  }
  .name {
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--color-fg-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .summary {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    font-size: var(--text-xs);
    line-height: 1.4;
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .chips {
    display: flex;
    gap: 0.375rem;
    padding-top: 0.15rem;
  }
</style>
