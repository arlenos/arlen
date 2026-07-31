<script lang="ts">
  /// Installed: the quiet list of what is on the machine, one row per app with
  /// the installed variant's source as prose. Managing (uninstall, variants)
  /// lives on the app page - one job per surface.
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import StoreHeader from "$lib/components/StoreHeader.svelte";
  import StoreRail from "$lib/components/StoreRail.svelte";
  import { t } from "$lib/i18n/messages";
  import { apps, catalogMocked, loadCatalog, isInstalled } from "$lib/stores/catalog";

  onMount(loadCatalog);

  const installed = $derived($apps.filter(isInstalled));

  function sourceLabel(s: string): string {
    return s === "forage" ? $t("st.src.forage") : s === "flathub" ? $t("st.src.flathub") : $t("st.src.debian");
  }
</script>

<div class="st-app">
  <StoreHeader />
  <div class="st-body">
    <StoreRail />
    <main class="st-main">
      <div class="st-content">
        {#if $catalogMocked}
          <p class="sample">{$t("st.sample")}</p>
        {/if}

        {#if installed.length === 0}
          <p class="quiet">{$t("st.inst.empty")}</p>
        {:else}
          {#each installed as app (app.id)}
            {@const v = app.variants.find((x) => x.installed)}
            <button type="button" class="row" id={`inst-${app.id}`} onclick={() => goto(`/app/${app.id}`)}>
              <span class="tile" style="background:{app.icon}" aria-hidden="true"></span>
              <span class="row-body">
                <span class="row-name">{app.name}</span>
                <span class="row-meta">{v ? sourceLabel(v.source) : ""}</span>
              </span>
            </button>
          {/each}
        {/if}
      </div>
    </main>
  </div>
</div>

<style>
  .st-app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: var(--color-bg-app);
    color: var(--color-fg-primary);
  }
  .st-body {
    flex: 1;
    min-height: 0;
    display: flex;
  }
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
  .tile {
    flex-shrink: 0;
    width: 2.5rem;
    height: 2.5rem;
    border-radius: var(--radius-input);
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
