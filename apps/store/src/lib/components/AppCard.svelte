<script lang="ts">
  /// The one app card, reused everywhere (collections and results): icon tile,
  /// name, one-line summary. Trust is the only chip and only when it has
  /// something to say - curated stays silent, community is flagged (§3); the
  /// package format never shows in browse.
  import { Badge } from "@arlen/ui-kit/components/ui/badge";
  import { t } from "$lib/i18n/messages";
  import { trustOf, type StoreApp } from "$lib/stores/catalog";

  let { app, onopen }: { app: StoreApp; onopen: (id: string) => void } = $props();
</script>

<button type="button" class="card" id={`app-${app.id}`} onclick={() => onopen(app.id)}>
  <span class="tile" style="background:{app.icon}" aria-hidden="true"></span>
  <span class="body">
    <span class="name">{app.name}</span>
    <span class="summary">{app.summary}</span>
    {#if trustOf(app) === "community"}
      <span class="chips"><Badge variant="outline">{$t("st.trust.community")}</Badge></span>
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
    text-align: left;
    cursor: pointer;
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
  .tile {
    flex-shrink: 0;
    width: 3rem;
    height: 3rem;
    border-radius: var(--radius-input);
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
  }
  .summary {
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
