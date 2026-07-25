<script lang="ts">
  /// The app page (store-app.md §5): led by the capability panel, not stars. The
  /// order is the message - what it can reach first (negatives carry the
  /// least-privilege story), then observed-vs-declared on your own machine, then
  /// the quiet trust panel (ODRS is one row, never the headline), screenshots,
  /// description, and the passive support link.
  import { onMount } from "svelte";
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import { ArrowLeft, ExternalLink, Check, Minus } from "lucide-svelte";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Badge } from "@arlen/ui-kit/components/ui/badge";
  import StoreHeader from "$lib/components/StoreHeader.svelte";
  import { t } from "$lib/i18n/messages";
  import {
    apps,
    catalogMocked,
    loadCatalog,
    trustFor,
    observedFor,
    installApp,
    type TrustSignals,
    type ObservedLine,
  } from "$lib/stores/catalog";

  const id = $derived($page.params.id);
  const app = $derived($apps.find((a) => a.id === id) ?? null);

  let trust = $state<TrustSignals | null>(null);
  let observed = $state<ObservedLine[]>([]);

  onMount(async () => {
    await loadCatalog();
  });

  // Load the per-app reads once the catalogue entry is there.
  $effect(() => {
    const a = app;
    if (!a) return;
    void trustFor(a.id).then((v) => (trust = v));
    if (a.installed) void observedFor(a.id).then((v) => (observed = v));
    else observed = [];
  });
</script>

<div class="st-app">
  <StoreHeader />

  <main class="st-main">
    {#if $catalogMocked}
      <p class="sample">{$t("st.sample")}</p>
    {/if}

    <button type="button" class="back" id="back" onclick={() => goto("/")}>
      <ArrowLeft size={15} strokeWidth={2} />
      {$t("st.back")}
    </button>

    {#if app}
      <header class="head">
        <span class="tile" style="background:{app.icon}" aria-hidden="true"></span>
        <div class="head-text">
          <h1 class="name">{app.name}</h1>
          <p class="summary">{app.summary}</p>
          <span class="chips"><Badge variant="outline">{$t(`st.tier.${app.tier}`)}</Badge></span>
        </div>
        <span class="head-action">
          {#if app.installed}
            <Badge variant="success" class="h-control px-3">{$t("st.installed")}</Badge>
          {:else}
            <Button id="install" onclick={() => installApp(app.id)}>{$t("st.install")}</Button>
          {/if}
        </span>
      </header>

      <section class="panel" aria-labelledby="reach-label">
        <div class="panel-label" id="reach-label">{$t("st.reach.title")}</div>
        {#each app.caps as cap (cap.text)}
          <div class="cap" class:negative={cap.negative}>
            {#if cap.negative}
              <Minus size={14} strokeWidth={2} aria-hidden="true" />
            {:else}
              <Check size={14} strokeWidth={2} aria-hidden="true" />
            {/if}
            <span>{cap.text}</span>
          </div>
        {/each}
        {#if app.enrolledDeb}
          <p class="cap-note">{$t("st.reach.confined")}</p>
        {/if}
      </section>

      {#if observed.length > 0}
        <section class="panel" aria-labelledby="observed-label">
          <div class="panel-label" id="observed-label">{$t("st.observed.title")}</div>
          {#each observed as line (line.text)}
            <p class="observed-line">{line.text}</p>
          {/each}
        </section>
      {/if}

      {#if trust}
        <section class="panel" aria-labelledby="trust-label">
          <div class="panel-label" id="trust-label">{$t("st.trust.title")}</div>
          <div class="trust-row">
            <span class="trust-k">{$t("st.trust.reproducible")}</span>
            <span class="trust-v">{trust.reproducible ? $t("st.trust.reproducible.yes") : $t("st.trust.reproducible.no")}</span>
          </div>
          <div class="trust-row">
            <span class="trust-k">{$t("st.trust.publisher")}</span>
            <span class="trust-v">{trust.verifiedPublisher ? $t("st.trust.publisher.yes") : $t("st.trust.publisher.no")}</span>
          </div>
          {#if trust.installCount != null}
            <div class="trust-row">
              <span class="trust-k">{$t("st.trust.installs")}</span>
              <span class="trust-v">{$t("st.trust.installs.value", { n: trust.installCount.toLocaleString() })}</span>
            </div>
          {/if}
          {#if trust.odrsRating != null}
            <div class="trust-row">
              <span class="trust-k">{$t("st.trust.rating")}</span>
              <span class="trust-v">{$t("st.trust.rating.value", { score: trust.odrsRating.toFixed(1) })}</span>
            </div>
          {/if}
        </section>
      {/if}

      {#if app.shots && app.shots.length > 0}
        <div class="group-label">{$t("st.screenshots")}</div>
        <div class="shots">
          {#each app.shots as shot, i (i)}
            <span class="shot" style="background:{shot}" aria-hidden="true"></span>
          {/each}
        </div>
      {/if}

      {#if app.description}
        <div class="group-label">{$t("st.about")}</div>
        <p class="desc">{app.description}</p>
      {/if}

      {#if app.donationUrl}
        <Button variant="ghost" class="gap-2 px-3 font-normal text-muted-foreground hover:text-foreground" id="support">
          <ExternalLink size={15} strokeWidth={1.75} />
          {$t("st.support")}
        </Button>
      {/if}
    {:else}
      <p class="quiet">{$t("st.notFound")}</p>
    {/if}
  </main>
</div>

<style>
  .st-app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: var(--color-bg-app);
    color: var(--color-fg-primary);
  }
  .st-main {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
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
  .back {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    margin-bottom: 1rem;
    padding: 0.3rem 0.6rem;
    border: none;
    border-radius: var(--radius-input);
    background: transparent;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
    cursor: pointer;
  }
  .back:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 6%, transparent);
    color: var(--color-fg-primary);
  }

  .head {
    display: flex;
    gap: 1rem;
    align-items: flex-start;
    margin-bottom: 1.25rem;
  }
  .tile {
    flex-shrink: 0;
    width: 4rem;
    height: 4rem;
    border-radius: var(--radius-card);
  }
  .head-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }
  .name {
    margin: 0;
    font-size: var(--text-xl);
    font-weight: 600;
  }
  .summary {
    margin: 0;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
  }
  .chips {
    display: flex;
    gap: 0.375rem;
    padding-top: 0.2rem;
  }
  .head-action {
    flex-shrink: 0;
  }

  /* Panels: one bordered card per story block, the flat house card language. */
  .panel {
    margin-bottom: 1rem;
    padding: 0.9rem 1rem 1rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    border-radius: var(--radius-card);
    background: color-mix(in srgb, var(--color-fg-primary) 2%, transparent);
  }
  .panel-label {
    margin-bottom: 0.6rem;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }

  .cap {
    display: flex;
    align-items: flex-start;
    gap: 0.55rem;
    padding: 0.3rem 0;
    font-size: var(--text-sm);
    line-height: 1.45;
    color: color-mix(in srgb, var(--color-fg-primary) 85%, transparent);
  }
  .cap :global(svg) {
    flex-shrink: 0;
    margin-top: 0.2rem;
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .cap.negative {
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
  }
  .cap-note {
    margin: 0.5rem 0 0;
    font-size: var(--text-xs);
    line-height: 1.45;
    color: color-mix(in srgb, var(--color-fg-primary) 52%, transparent);
  }

  .observed-line {
    margin: 0;
    padding: 0.25rem 0;
    font-size: var(--text-sm);
    line-height: 1.45;
    color: color-mix(in srgb, var(--color-fg-primary) 78%, transparent);
  }

  .trust-row {
    display: flex;
    gap: 1rem;
    padding: 0.3rem 0;
    font-size: var(--text-sm);
  }
  .trust-k {
    flex: 0 0 11rem;
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .trust-v {
    flex: 1;
    color: color-mix(in srgb, var(--color-fg-primary) 82%, transparent);
  }

  .group-label {
    margin: 1.25rem 0 0.6rem;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .shots {
    display: flex;
    gap: 0.6rem;
    overflow-x: auto;
    padding-bottom: 0.25rem;
  }
  .shot {
    flex-shrink: 0;
    width: 16rem;
    aspect-ratio: 16 / 10;
    border-radius: var(--radius-input);
  }
  .desc {
    margin: 0 0 1rem;
    font-size: var(--text-sm);
    line-height: 1.55;
    color: color-mix(in srgb, var(--color-fg-primary) 78%, transparent);
  }
  .quiet {
    margin: 0;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
</style>
