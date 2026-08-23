<script lang="ts">
  /// Installed: the quiet list of what is on the machine, one row per app with
  /// the installed variant's source as prose. Managing (uninstall, variants)
  /// lives on the app page - one job per surface.
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n/messages";
  import IconTile from "$lib/components/IconTile.svelte";
  import { apps, catalogMocked, loadCatalog, type Tier } from "$lib/stores/catalog";

  onMount(loadCatalog);

  const installed = $derived($apps.filter((a) => a.installed));

  function sourceLabel(s: Tier): string {
    return s === "forage"
      ? $t("st.src.forage")
      : s === "flathub"
        ? $t("st.src.flathub")
        : s === "debian"
          ? $t("st.src.debian")
          : $t("st.src.native");
  }
</script>

<main class="st-main">
  <div class="st-content">
    {#if $catalogMocked}
      <p class="sample">{$t("st.sample")}</p>
    {/if}

    {#if installed.length === 0}
      <p class="quiet">{$t("st.inst.empty")}</p>
    {:else}
      {#each installed as app (app.id)}
        <button type="button" class="row" id={`inst-${app.id}`} onclick={() => goto(`/app/${app.id}`)}>
          <IconTile icon={app.icon} name={app.name} size="2.5rem" />
          <span class="row-body">
            <span class="row-name">{app.name}</span>
            <span class="row-meta">{sourceLabel(app.tier)}</span>
          </span>
        </button>
      {/each}
    {/if}
  </div>
</main>

<style>
  .st-main {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
  }
  .st-content {
    width: 100%;
    max-width: 46rem;
    margin: 0 auto;
    padding: 1.25rem 1.5rem 2rem;
  }
  .sample {
    margin: 0 0 0.75rem;
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .quiet {
    margin: 0;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .row {
    display: flex;
    gap: 0.75rem;
    align-items: center;
    width: 100%;
    margin-bottom: 0.4rem;
    padding: 0.6rem 0.75rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    border-radius: var(--radius-card);
    background: color-mix(in srgb, var(--color-fg-primary) 2%, transparent);
    text-align: start;
    cursor: pointer;
    transition: background var(--duration-fast, 150ms) ease;
  }
  .row:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 5%, transparent);
  }
  .row-body {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    min-width: 0;
  }
  .row-name {
    font-size: var(--text-sm);
    font-weight: 600;
  }
  .row-meta {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 52%, transparent);
  }
</style>
