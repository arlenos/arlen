<script lang="ts">
  /// The app page (store-app.md §5): led by the capability panel, not stars. The
  /// order is the message - what it can reach first (negatives carry the
  /// least-privilege story), then observed-vs-declared on your own machine, then
  /// the quiet trust panel (ODRS is one row, never the headline), screenshots,
  /// description, and the passive support link.
  import { onMount } from "svelte";
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import { ArrowLeft, Check, Minus } from "lucide-svelte";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Badge } from "@arlen/ui-kit/components/ui/badge";
  import { ConfirmDialog } from "@arlen/ui-kit/components/ui/confirm-dialog";
  import { t } from "$lib/i18n/messages";
  import { reachRows } from "$lib/caps";
  import IconTile from "$lib/components/IconTile.svelte";
  import {
    apps,
    catalogMocked,
    loadCatalog,
    trustFor,
    observedFor,
    installApp,
    uninstallApp,
    uninstallStatus,
    trustOf,
    layerFor,
    type LayerSignals,
    type ObservedStatus,
    type SourceLayer,
    type StoreVariant,
    type Tier,
  } from "$lib/stores/catalog";
  import { pendingUpdates, loadUpdates } from "$lib/stores/updates";

  const id = $derived($page.params.id);
  // An update waiting for this app; the Updates page owns the decision, this
  // head only points at it.
  const pending = $derived($pendingUpdates.some((u) => u.id === id));
  const removal = $derived(id ? $uninstallStatus[id] : undefined);
  let confirmUninstall = $state(false);
  const app = $derived($apps.find((a) => a.id === id) ?? null);

  // The chosen install variant; the capability panel below follows it, so
  // "install the least-privilege variant" is a real, visible choice (§9.1).
  let chosen = $state(0);
  const variant = $derived(app ? (app.variants[chosen] ?? app.variants[0]) : null);
  const leastWeight = $derived(app ? Math.min(...app.variants.map((v) => v.capWeight)) : 0);

  let trust = $state<LayerSignals>([]);
  let observed = $state<ObservedStatus | null>(null);

  onMount(async () => {
    void loadUpdates();
    await loadCatalog();
  });

  // Load the per-app reads once the catalogue entry is there.
  $effect(() => {
    const a = app;
    if (!a) return;
    chosen = a.defaultVariant;
    void trustFor(a.id).then((v) => (trust = v));
    if (a.installed) void observedFor(a.id).then((v) => (observed = v));
    else observed = null;
  });

  function sourceLabel(s: Tier): string {
    return s === "forage"
      ? $t("st.src.forage")
      : s === "flathub"
        ? $t("st.src.flathub")
        : s === "debian"
          ? $t("st.src.debian")
          : $t("st.src.native");
  }

  // Which layers a tier can stand for; the trust panel row follows the chosen
  // variant through this map.
  const TIER_LAYERS: Record<Tier, SourceLayer[]> = {
    forage: ["Personal", "Community", "Official"],
    flathub: ["Flatpak"],
    debian: ["Apt"],
    installed: ["Native"],
  };
  const signals = $derived.by(() => {
    if (!variant) return null;
    const layers = TIER_LAYERS[variant.source];
    return trust.find(([layer]) => layers.includes(layer))?.[1] ?? null;
  });

  // The variant row's one meta line: the same axes on every row (version and
  // capability count) so the rows actually compare, then the least-privilege
  // marker where it is earned.
  function variantMeta(v: StoreVariant): string {
    const a = app;
    if (!a) return "";
    const parts: string[] = [];
    if (v.version) parts.push($t("st.version.value", { v: v.version }));
    parts.push($t("st.capCount", { n: v.capabilities.length }));
    const differ = a.variants.some((o) => o.capWeight !== leastWeight);
    // Not lowercased into the sentence: German capitalises its nouns, so the
    // catalogue string is used exactly as written.
    if (v.capWeight === leastWeight && differ) parts.push($t("st.leastPrivilege"));
    return parts.join(", ");
  }

  const paintable = (s: string) => s.startsWith("linear-gradient(") || /^https?:\/\//.test(s);
  const shots = $derived(app ? app.screenshots.filter(paintable) : []);
</script>

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
        <IconTile icon={app.icon} name={app.name} size="4rem" />
        <div class="head-text">
          <h1 class="name">{app.name}</h1>
          <p class="summary">{app.summary}</p>
          {#if trustOf(app) === "community"}
            <span class="chips"><Badge variant="outline">{$t("st.trust.community")}</Badge></span>
          {/if}
        </div>
        <span class="head-action">
          {#if app.installed}
            <!-- Installed is a state with two acts beside it, not a dead badge:
                 the update (decided on the Updates page) and the removal. -->
            <Badge variant="success" class="h-control px-3">{$t("st.installed")}</Badge>
            {#if pending}
              <Button variant="outline" id="update-available" onclick={() => goto("/updates")}>{$t("st.app.updateAvailable")}</Button>
            {/if}
            <Button
              variant="outline"
              id="uninstall"
              disabled={removal?.kind === "removing"}
              onclick={() => (confirmUninstall = true)}
            >
              {$t("st.app.uninstall")}
            </Button>
          {:else if app.installable}
            <Button id="install" onclick={() => installApp(app.id, layerFor(app, chosen))}>{$t("st.install")}</Button>
          {/if}
        </span>
      </header>

      {#if removal?.kind === "removing"}
        <p class="quiet">{$t("st.app.removing")}</p>
      {:else if removal?.kind === "started"}
        <p class="quiet">{$t("st.app.removalStarted")}</p>
      {:else if removal?.kind === "refused"}
        <!-- The daemon's own refusal, in place: a desktop app it will not
             remove, or a layer this build cannot remove. It is built to be
             offered Remove and to say no; the page shows the no. -->
        <p class="refused" role="alert">{$t("st.app.uninstallRefused", { reason: removal.reason })}</p>
      {/if}

      {#if !app.installable && !app.installed}
        <p class="quiet">{$t("st.notInstallable")}</p>
      {/if}

      {#if app.variants.length > 1}
        <section class="panel" aria-labelledby="variants-label">
          <div class="panel-label" id="variants-label">{$t("st.installFrom")}</div>
          <div class="variants" role="radiogroup" aria-label={$t("st.installFrom")}>
            {#each app.variants as v, i (i)}
              <button
                type="button"
                class="variant"
                class:on={chosen === i}
                role="radio"
                aria-checked={chosen === i}
                id={`variant-${i}-${v.source}`}
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
        {#each reachRows($t, variant.capabilities) as cap (cap.text)}
          <div class="cap" class:negative={cap.negative}>
            {#if cap.negative}
              <Minus size={14} strokeWidth={2} aria-hidden="true" />
            {:else}
              <Check size={14} strokeWidth={2} aria-hidden="true" />
            {/if}
            <span>{cap.text}</span>
          </div>
        {/each}
        {#if variant.source === "debian"}
          <p class="cap-note">{$t("st.reach.confined")}</p>
        {/if}
      </section>

      {#if app.installed && observed}
        <section class="panel" aria-labelledby="observed-label">
          <div class="panel-label" id="observed-label">{$t("st.observed.title")}</div>
          {#if observed.state === "measured"}
            <p class="observed-line">{$t("st.observed.window", { days: observed.windowDays })}</p>
            {#each observed.declared as cap (cap)}
              {@const used = observed.observed.includes(cap)}
              <div class="cap" class:negative={!used}>
                {#if used}
                  <Check size={14} strokeWidth={2} aria-hidden="true" />
                {:else}
                  <Minus size={14} strokeWidth={2} aria-hidden="true" />
                {/if}
                <span
                  >{reachRows($t, [cap])[0].text}:
                  {used ? $t("st.observed.used") : $t("st.observed.notObserved")}</span
                >
              </div>
            {/each}
          {:else}
            <!-- "Unavailable" is its own state, said plainly - an empty panel
                 here would read as a clean bill of health the system cannot
                 give (§8.2). -->
            <p class="observed-line">{$t("st.observed.unavailable")}</p>
          {/if}
        </section>
      {/if}

      <section class="panel" aria-labelledby="trust-label">
        <div class="panel-label" id="trust-label">{$t("st.trust.title")}</div>
        <!-- Reproducible and verified are facts about the CHOSEN variant, so
             they read from it and follow the selection like the capability
             panel does. Installs, the ODRS rating and the supply chain come
             from the same variant's source layer; a signal the layer does not
             attest is hidden, never shown empty (§9.2). -->
        <div class="trust-row">
          <span class="trust-k">{$t("st.trust.reproducible")}</span>
          <span class="trust-v">{variant.reproducible ? $t("st.trust.reproducible.yes") : $t("st.trust.reproducible.no")}</span>
        </div>
        <div class="trust-row">
          <span class="trust-k">{$t("st.trust.publisher")}</span>
          <span class="trust-v">{variant.verified ? $t("st.trust.publisher.yes") : $t("st.trust.publisher.no")}</span>
        </div>
        {#if signals?.attestation}
          <div class="trust-row">
            <span class="trust-k">{$t("st.trust.attested")}</span>
            <span class="trust-v"
              >{$t("st.trust.attested.value", { signer: signals.attestation.signer })}
              ({signals.attestation.pinned_here ? $t("st.trust.attested.pinned") : $t("st.trust.attested.unpinned")})</span
            >
          </div>
        {/if}
        {#if signals?.install_count != null}
          <div class="trust-row">
            <span class="trust-k">{$t("st.trust.installs")}</span>
            <span class="trust-v">{$t("st.trust.installs.value", { n: signals.install_count.toLocaleString() })}</span>
          </div>
        {/if}
        {#if signals?.odrs_score != null}
          <div class="trust-row">
            <span class="trust-k">{$t("st.trust.rating")}</span>
            <span class="trust-v">{$t("st.trust.rating.value", { score: signals.odrs_score.toFixed(1) })}</span>
          </div>
        {/if}
      </section>

      {#if shots.length > 0}
        <div class="group-label">{$t("st.screenshots")}</div>
        <div class="shots">
          {#each shots as shot, i (i)}
            {#if shot.startsWith("linear-gradient(")}
              <span class="shot" style="background:{shot}" aria-hidden="true"></span>
            {:else}
              <img class="shot" src={shot} alt="" loading="lazy" />
            {/if}
          {/each}
        </div>
      {/if}

      {#if app.description}
        <div class="group-label">{$t("st.about")}</div>
        <p class="desc">{app.description}</p>
      {/if}
    {:else}
      <p class="quiet">{$t("st.notFound")}</p>
    {/if}
  </div>
</main>

<ConfirmDialog
  open={confirmUninstall}
  title={$t("st.app.uninstallTitle", { name: app?.name ?? "" })}
  message={$t("st.app.uninstallMsg")}
  confirmLabel={$t("st.app.uninstall")}
  variant="destructive"
  onConfirm={async () => {
    confirmUninstall = false;
    if (app) await uninstallApp(app.id);
  }}
  onCancel={() => (confirmUninstall = false)}
/>

<style>
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
    object-fit: cover;
    background: color-mix(in srgb, var(--color-fg-primary) 5%, transparent);
  }
  .desc {
    margin: 0 0 1rem;
    font-size: var(--text-sm);
    line-height: 1.55;
    color: color-mix(in srgb, var(--color-fg-primary) 78%, transparent);
  }
  .refused {
    margin: 0 0 0.75rem;
    font-size: var(--text-sm);
    color: var(--color-error, #dc2626);
  }
  .quiet {
    margin: 0 0 1rem;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
</style>
