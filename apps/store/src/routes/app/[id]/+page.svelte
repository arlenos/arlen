<script lang="ts">
  /// The app page (store-app.md §5): led by the capability panel, not stars. The
  /// order is the message - what it can reach first (negatives carry the
  /// least-privilege story), then observed-vs-declared on your own machine, then
  /// the quiet trust panel (ODRS is one row, never the headline), screenshots,
  /// description, and the passive support link.
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
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
    trustOf,
    isInstalled,
    type TrustSignals,
    type ObservedLine,
  } from "$lib/stores/catalog";

  const id = $derived($page.params.id);
  const app = $derived($apps.find((a) => a.id === id) ?? null);

  // The chosen install variant; the capability panel below follows it, so
  // "install the least-privilege variant" is a real, visible choice (§9.1).
  let chosen = $state(0);
  const variant = $derived(app ? (app.variants[chosen] ?? app.variants[0]) : null);
  const leastWeight = $derived(app ? Math.min(...app.variants.map((v) => v.capWeight)) : 0);

  let trust = $state<TrustSignals | null>(null);
  let observed = $state<ObservedLine[]>([]);

  onMount(async () => {
    await loadCatalog();
  });

  // Load the per-app reads once the catalogue entry is there.
  $effect(() => {
    const a = app;
    if (!a) return;
    chosen = a.defaultVariant;
    void trustFor(a.id).then((v) => (trust = v));
    if (isInstalled(a)) void observedFor(a.id).then((v) => (observed = v));
    else observed = [];
  });

  function sourceLabel(s: string): string {
    return s === "forage" ? $t("st.src.forage") : s === "flathub" ? $t("st.src.flathub") : $t("st.src.debian");
  }

  // The variant row's one meta line: the same axis on every row (the
  // capability count) so the rows actually compare, then the least-privilege
  // marker where it is earned, then installed.
  function variantMeta(v: (typeof app extends null ? never : NonNullable<typeof app>)["variants"][number]): string {
    const a = app;
    if (!a) return "";
    const parts: string[] = [$t("st.capCount", { n: v.caps.filter((c) => !c.negative).length })];
    const differ = a.variants.some((o) => o.capWeight !== leastWeight);
    if (v.capWeight === leastWeight && differ) parts.push($t("st.leastPrivilege").toLowerCase());
    if (v.installed) parts.push($t("st.installed").toLowerCase());
    return parts.join(", ");
  }
</script>

<div class="st-app">
  <StoreHeader />

  <main class="st-main">
  <div class="st-content">
    {#if $catalogMocked}
      <p class="sample">{$t("st.sample")}</p>
    {/if}

    <button type="button" class="back" id="back" onclick={() => goto("/")}>
      <ArrowLeft size={15} strokeWidth={2} />
      {$t("st.back")}
    </button>

    {#if app && variant}
      <header class="head">
        <span class="tile" style="background:{app.icon}" aria-hidden="true"></span>
        <div class="head-text">
          <h1 class="name">{app.name}</h1>
          <p class="summary">{app.summary}</p>
          {#if trustOf(app) === "community"}
            <span class="chips"><Badge variant="outline">{$t("st.trust.community")}</Badge></span>
          {/if}
        </div>
        <span class="head-action">
          {#if variant.installed}
            <Badge variant="success" class="h-control px-3">{$t("st.installed")}</Badge>
          {:else}
            <Button id="install" onclick={() => installApp(app.id, variant.source)}>{$t("st.install")}</Button>
          {/if}
        </span>
      </header>

      {#if app.variants.length > 1}
        <section class="panel" aria-labelledby="variants-label">
          <div class="panel-label" id="variants-label">{$t("st.installFrom")}</div>
          <div class="variants" role="radiogroup" aria-label={$t("st.installFrom")}>
            {#each app.variants as v, i (v.source)}
              <button
                type="button"
                class="variant"
                class:on={chosen === i}
                role="radio"
                aria-checked={chosen === i}
                id={`variant-${v.source}`}
                onclick={() => (chosen = i)}
              >
                <span class="variant-name">{sourceLabel(v.source)}</span>
                <span class="variant-meta">{variantMeta(v)}</span>
                {#if v.trust === "community"}
                  <Badge variant="outline">{$t("st.trust.community")}</Badge>
                {/if}
              </button>
            {/each}
          </div>
        </section>
      {/if}

      <section class="panel" aria-labelledby="reach-label">
        <div class="panel-label" id="reach-label">{$t("st.reach.title")}</div>
        {#each variant.caps as cap (cap.text)}
          <div class="cap" class:negative={cap.negative}>
            {#if cap.negative}
              <Minus size={14} strokeWidth={2} aria-hidden="true" />
            {:else}
              <Check size={14} strokeWidth={2} aria-hidden="true" />
            {/if}
            <span>{cap.text}</span>
          </div>
        {/each}
        {#if variant.enrolledDeb}
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

      <section class="panel" aria-labelledby="trust-label">
        <div class="panel-label" id="trust-label">{$t("st.trust.title")}</div>
        <!-- Reproducible and verified are facts about the CHOSEN variant (the
             fixture already differs per source), so they read from it and
             follow the selection like the capability panel does. Installs and
             the ODRS rating are app-level. -->
        <div class="trust-row">
          <span class="trust-k">{$t("st.trust.reproducible")}</span>
          <span class="trust-v">{variant.reproducible ? $t("st.trust.reproducible.yes") : $t("st.trust.reproducible.no")}</span>
        </div>
        <div class="trust-row">
          <span class="trust-k">{$t("st.trust.publisher")}</span>
          <span class="trust-v">{variant.verified ? $t("st.trust.publisher.yes") : $t("st.trust.publisher.no")}</span>
        </div>
        {#if trust?.installCount != null}
          <div class="trust-row">
            <span class="trust-k">{$t("st.trust.installs")}</span>
            <span class="trust-v">{$t("st.trust.installs.value", { n: trust.installCount.toLocaleString() })}</span>
          </div>
        {/if}
        {#if trust?.odrsRating != null}
          <div class="trust-row">
            <span class="trust-k">{$t("st.trust.rating")}</span>
            <span class="trust-v">{$t("st.trust.rating.value", { score: trust.odrsRating.toFixed(1) })}</span>
          </div>
        {/if}
      </section>

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
        {@const url = app.donationUrl}
        <Button
          variant="ghost"
          class="gap-2 px-3 font-normal text-muted-foreground hover:text-foreground"
          id="support"
          onclick={() => invoke("open_url", { url }).catch(() => {})}
        >
          <ExternalLink size={15} strokeWidth={1.75} />
          {$t("st.support")}
        </Button>
      {/if}
    {:else}
      <p class="quiet">{$t("st.notFound")}</p>
    {/if}
  </div>
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
  /* The scroller spans the window so the scrollbar sits at the edge; the
     content column is capped inside it. */
  .st-main {
    flex: 1;
    min-height: 0;
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

  /* The install-variant rows: the established selection language (border +
     wash on the active one). The source is the choice here, so it is named. */
  .variants {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }
  .variant {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.55rem 0.7rem;
    border: 1px solid transparent;
    border-radius: var(--radius-input);
    background: transparent;
    text-align: start;
    cursor: pointer;
  }
  .variant:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 5%, transparent);
  }
  .variant.on {
    border-color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
    background: color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
  }
  .variant-name {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--color-fg-primary);
  }
  .variant-meta {
    flex: 1;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 52%, transparent);
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
