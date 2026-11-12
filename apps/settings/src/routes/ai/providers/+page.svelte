<script lang="ts">
  /// AI providers (ai-providers-plan.md §Settings): the branded list of
  /// providers the proxy knows, with what each one is and where it sits, plus
  /// the escape hatch to add a custom one. Local and EU-sovereign providers are
  /// featured first. Choosing which model answers lives on the Default models
  /// page. The catalogue comes from the proxy (`ai_providers_list`); a row
  /// carries no switch and no test, because nothing serves either.
  import { onMount } from "svelte";
  import { writable } from "svelte/store";
  import { invoke } from "@tauri-apps/api/core";
  import { tauriAvailable } from "$lib/tauri";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { Notice } from "@arlen/ui-kit/components/ui/notice";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Section } from "@arlen/ui-kit/components/ui/section";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Badge } from "@arlen/ui-kit/components/ui/badge";
  import * as Tooltip from "@arlen/ui-kit/components/ui/tooltip";
  import { ProviderLogo } from "@arlen/ui-kit/components/ui/provider-logo";
  import { Plus } from "lucide-svelte";
  import { t } from "$lib/i18n/messages";
  import AddProviderDialog from "$lib/components/AddProviderDialog.svelte";

  /// One provider as the catalogue + the broker report it. `configured` means
  /// credentials exist (or none are needed, for a local provider). `region` is
  /// an optional sovereignty note when the catalogue carries one.
  interface Provider {
    id: string;
    name: string;
    kind: "local" | "cloud";
    region?: string;
    configured: boolean;
    /// How you connect: a per-token key, your existing subscription, or a free
    /// tier with no login. From the catalogue (`authMethod`); shapes the chip.
    authMethod?: "api-key" | "subscription-login" | "free";
    /// A reverse-engineered access path (no official API). Carries a warning:
    /// it may break or risk account suspension.
    unofficial?: boolean;
    /// Where the servers sit, for the sovereignty info line. Hand-curated (not
    /// from models.dev), governs the jurisdiction chip + its law tooltip.
    jurisdiction?: "eu" | "us" | "cn";
    /// Training posture: "no" (clean), "no-paid" (paid API only, the free tier
    /// trains, shown with a `*`), or "yes".
    trainsOnYou?: "no" | "no-paid" | "yes";
    /// Whether the served model has open weights, the escape hatch: you can run
    /// it yourself, so a cloud host is an interchangeable vendor not lock-in.
    openWeight?: boolean;
  }

  // The catalogue is loaded from the daemon. The Svelte-5 IPC caveat applies
  // (state mutated from a Tauri callback does not re-render reliably), so the
  // list lives in a writable store. `loaded` separates "still reading" from a
  // genuinely empty catalogue, so the page never lies about either.
  const providers = writable<Provider[]>([]);
  let loaded = $state(false);

  function parse<T>(json: string, fallback: T): T {
    try {
      return JSON.parse(json) as T;
    } catch {
      return fallback;
    }
  }

  // A dev sample so the surface renders under vite (no daemon). Covers every
  // auth-method plus an unofficial reverse-engineered path. The live catalogue
  // replaces it the moment `ai_providers_list` answers.
  const DEV_FIXTURE: Provider[] = [
    { id: "ollama", name: "Ollama", kind: "local", configured: true },
    { id: "mistral", name: "Mistral", kind: "cloud", region: "EU", configured: true, authMethod: "api-key", jurisdiction: "eu", trainsOnYou: "no-paid", openWeight: true },
    { id: "claude", name: "Claude", kind: "cloud", configured: true, authMethod: "subscription-login", jurisdiction: "us", trainsOnYou: "no-paid", openWeight: false },
    { id: "groq", name: "Groq", kind: "cloud", configured: false, authMethod: "free", jurisdiction: "us", trainsOnYou: "no", openWeight: true },
    { id: "copilot", name: "GitHub Copilot", kind: "cloud", configured: false, authMethod: "subscription-login", unofficial: true, jurisdiction: "us", trainsOnYou: "no-paid", openWeight: false },
  ];

  async function loadProviders() {
    try {
      providers.set(parse<Provider[]>(await invoke<string>("ai_providers_list"), []));
    } catch {
      providers.set(!tauriAvailable ? DEV_FIXTURE : []);
    } finally {
      loaded = true;
    }
  }
  onMount(loadProviders);

  /// The plain-language label for how you connect to a provider.
  function authLabel(m: Provider["authMethod"]): string {
    switch (m) {
      case "subscription-login":
        return $t("s.prov.auth.subscription");
      case "free":
        return $t("s.prov.auth.free");
      case "api-key":
        return $t("s.prov.auth.apiKey");
      default:
        return "";
    }
  }

  /// The sovereignty info line: factual chips, offer-never-shame. Each chip
  /// carries a tooltip so the facts stay honest (the training `*`, the law
  /// behind a jurisdiction). Local needs none, it is the strongest case.
  type SovChip = { label: string; tip: string };
  function sovereignChips(p: Provider): SovChip[] {
    if (p.kind === "local") return [];
    const chips: SovChip[] = [];
    if (p.jurisdiction === "eu")
      chips.push({ label: $t("s.prov.jur.eu"), tip: $t("s.prov.jur.eu.tip") });
    else if (p.jurisdiction === "us")
      chips.push({ label: $t("s.prov.jur.us"), tip: $t("s.prov.jur.us.tip") });
    else if (p.jurisdiction === "cn")
      chips.push({ label: $t("s.prov.jur.cn"), tip: $t("s.prov.jur.cn.tip") });
    if (p.trainsOnYou === "no")
      chips.push({ label: $t("s.prov.train.no"), tip: $t("s.prov.train.no.tip") });
    else if (p.trainsOnYou === "no-paid")
      chips.push({ label: $t("s.prov.train.noPaid"), tip: $t("s.prov.train.noPaid.tip") });
    else if (p.trainsOnYou === "yes")
      chips.push({ label: $t("s.prov.train.yes"), tip: $t("s.prov.train.yes.tip") });
    if (p.openWeight)
      chips.push({ label: $t("s.prov.openWeights"), tip: $t("s.prov.openWeights.tip") });
    return chips;
  }

  function meta(p: Provider): string {
    if (p.kind === "local") return $t("s.prov.local");
    return p.region ? $t("s.prov.cloudRegion", { region: p.region }) : $t("s.prov.cloud");
  }

  // The escape hatch: add a provider the catalogue does not carry. The dialog owns
  // the form + the broker-gated save; reload the catalogue on close so a new one
  // shows once that backend lands.
  let showAdd = $state(false);
</script>

<Page
  title={$t("s.prov.title")}
  description={$t("s.prov.desc")}
>
  <SectionGrid>
    {#if !tauriAvailable}
      <Notice tone="neutral" class="span-full" text={$t("s.prov.sample")} />
    {/if}
    <Section class="span-full" id="ai-providers">
      {#each $providers as p (p.id)}
        {@render row(p)}
      {/each}
      {#if loaded && $providers.length === 0}
        <p class="empty">{$t("s.prov.empty")}</p>
      {/if}
    </Section>
    <div class="add-row span-full">
      <Button variant="secondary" size="sm" onclick={() => (showAdd = true)}>
        <Plus size={15} strokeWidth={2} />
        {$t("s.prov.add")}
      </Button>
    </div>
  </SectionGrid>
</Page>

<AddProviderDialog
  open={showAdd}
  onClose={() => {
    showAdd = false;
    loadProviders();
  }}
/>

{#snippet row(p: Provider)}
  <Row label={p.name} description={meta(p)} id={`provider-${p.id}`}>
    {#snippet leading()}
      <ProviderLogo id={p.id} name={p.name} size={24} />
    {/snippet}
    {#snippet preview()}
      {#if (p.kind === "cloud" && p.authMethod) || p.unofficial}
        <span class="chips">
          {#if p.kind === "cloud" && p.authMethod}
            <Badge variant="outline">{authLabel(p.authMethod)}</Badge>
          {/if}
          {#if p.unofficial}
            <Tooltip.Root>
              <Tooltip.Trigger>
                {#snippet child({ props })}
                  <span {...props} class="chip-trigger"><Badge variant="warn">{$t("s.prov.unofficial")}</Badge></span>
                {/snippet}
              </Tooltip.Trigger>
              <Tooltip.TooltipContent side="top">
                {$t("s.prov.unofficial.tip")}
              </Tooltip.TooltipContent>
            </Tooltip.Root>
          {/if}
        </span>
      {/if}
    {/snippet}
    {#snippet below()}
      {#if p.kind === "local"}
        <p class="sov-local">{$t("s.prov.runsLocal")}</p>
      {:else}
        {@const chips = sovereignChips(p)}
        {#if chips.length}
          <span class="sov">
            {#each chips as c (c.label)}
              <Tooltip.Root>
                <Tooltip.Trigger>
                  {#snippet child({ props })}
                    <span {...props} class="chip-trigger"><Badge variant="outline">{c.label}</Badge></span>
                  {/snippet}
                </Tooltip.Trigger>
                <Tooltip.TooltipContent side="top">{c.tip}</Tooltip.TooltipContent>
              </Tooltip.Root>
            {/each}
          </span>
        {/if}
      {/if}
    {/snippet}
  </Row>
{/snippet}

<style>
  .empty {
    margin: 0;
    padding: var(--space-row, 0.75rem) 1rem;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  /* The add escape hatch sits just under the catalogue, reading as secondary. */
  .add-row {
    display: flex;
    padding-top: 0.5rem;
  }
  /* The attribute chips (how you connect, plus any unofficial warning) sit in
     the row's preview slot, between the name and the control cluster. */
  .chips {
    display: inline-flex;
    align-items: center;
    gap: 0.375rem;
    /* WRAPS, so a narrow row stacks the badges instead of squeezing the provider's
       name out of existence. At 720px in German this row is the one that measured
       "GitHub Copilot" into 20px while these kept 155; the kit now shares the
       shortfall, and wrapping is how this side gives its share without any badge
       being cut in half. */
    flex-wrap: wrap;
    justify-content: flex-end;
  }
  .chip-trigger {
    display: inline-flex;
    cursor: default;
  }
  /* The sovereignty info line sits under the row, aligned with the label (past
     the 24px logo + its gap). Facts, offer-never-shame: quiet neutral chips. */
  .sov {
    display: flex;
    flex-wrap: wrap;
    gap: 0.375rem;
    padding-inline-start: 2.375rem;
  }
  .sov-local {
    margin: 0;
    padding-inline-start: 2.375rem;
    font-size: var(--text-2xs);
    color: var(--color-fg-secondary, #a1a1aa);
  }
</style>
