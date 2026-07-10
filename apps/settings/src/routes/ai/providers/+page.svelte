<script lang="ts">
  /// AI providers, the manager surface (ai-providers-plan.md §Settings): the
  /// branded list of providers with per-provider enable and a connection test,
  /// plus the escape hatch to add a custom one. Local and EU-sovereign
  /// providers are featured first (on-brand: sovereignty by default). Choosing
  /// which model answers lives on the separate Default models page; this page
  /// is the catalogue + login, not the live switch (that is the in-chat
  /// picker). The catalogue + per-provider enable come from the daemon
  /// (`ai_providers_list` / `ai_provider_set_enabled`); add + test are the
  /// broker-gated escape hatch, still mocked until that backend lands.
  import { onMount } from "svelte";
  import { writable } from "svelte/store";
  import { invoke } from "@tauri-apps/api/core";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Badge } from "@arlen/ui-kit/components/ui/badge";
  import * as Tooltip from "@arlen/ui-kit/components/ui/tooltip";
  import { ProviderLogo } from "@arlen/ui-kit/components/ui/provider-logo";
  import { Plus } from "lucide-svelte";
  import { t } from "$lib/i18n/messages";
  import AddProviderDialog from "$lib/components/AddProviderDialog.svelte";

  /// One provider as the catalogue + the broker report it. `configured` means
  /// credentials exist (or none are needed, for a local provider); an enabled
  /// provider that is not configured prompts to connect. `status` is the last
  /// connection-test outcome. `region` is an optional sovereignty note when the
  /// catalogue carries one.
  interface Provider {
    id: string;
    name: string;
    kind: "local" | "cloud";
    region?: string;
    enabled: boolean;
    configured: boolean;
    status: "ok" | "error" | "untested";
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
    { id: "ollama", name: "Ollama", kind: "local", enabled: true, configured: true, status: "ok" },
    { id: "mistral", name: "Mistral", kind: "cloud", region: "EU", enabled: true, configured: true, status: "ok", authMethod: "api-key", jurisdiction: "eu", trainsOnYou: "no-paid", openWeight: true },
    { id: "claude", name: "Claude", kind: "cloud", enabled: true, configured: true, status: "ok", authMethod: "subscription-login", jurisdiction: "us", trainsOnYou: "no-paid", openWeight: false },
    { id: "groq", name: "Groq", kind: "cloud", enabled: false, configured: false, status: "untested", authMethod: "free", jurisdiction: "us", trainsOnYou: "no", openWeight: true },
    { id: "copilot", name: "GitHub Copilot", kind: "cloud", enabled: false, configured: false, status: "untested", authMethod: "subscription-login", unofficial: true, jurisdiction: "us", trainsOnYou: "no-paid", openWeight: false },
  ];

  async function loadProviders() {
    try {
      providers.set(parse<Provider[]>(await invoke<string>("ai_providers_list"), []));
    } catch {
      providers.set(import.meta.env.DEV ? DEV_FIXTURE : []);
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

  /// Enable or disable a provider. Optimistic, reverted if the daemon does not
  /// return `ok` (the bridge returns a status string).
  async function setEnabled(id: string, value: boolean) {
    providers.update((list) => list.map((p) => (p.id === id ? { ...p, enabled: value } : p)));
    let ok = false;
    try {
      ok = (await invoke<string>("ai_provider_set_enabled", { id, enabled: value })) === "ok";
    } catch {
      ok = false;
    }
    if (!ok) {
      providers.update((list) =>
        list.map((p) => (p.id === id ? { ...p, enabled: !value } : p)),
      );
    }
  }

  function meta(p: Provider): string {
    if (p.kind === "local") return $t("s.prov.local");
    return p.region ? $t("s.prov.cloudRegion", { region: p.region }) : $t("s.prov.cloud");
  }

  // The connection test runs the real `ai_provider_test` (the proxy GETs the
  // provider's catalogued models endpoint). Per-provider so two rows can test
  // independently; the verdict shape is { ok, httpStatus?, network? }.
  type TestVerdict =
    | { state: "testing" }
    | { state: "ok" }
    | { state: "http"; status: number }
    | { state: "network" };
  const tests = writable<Record<string, TestVerdict>>({});

  // The escape hatch: add a provider the catalogue does not carry. The dialog owns
  // the form + the broker-gated save; reload the catalogue on close so a new one
  // shows once that backend lands.
  let showAdd = $state(false);

  async function testProvider(id: string) {
    tests.update((m) => ({ ...m, [id]: { state: "testing" } }));
    let verdict: TestVerdict = { state: "network" };
    try {
      const r = parse<{ ok?: boolean; httpStatus?: number; network?: string }>(
        await invoke<string>("ai_provider_test", { id }),
        {},
      );
      if (r.ok) verdict = { state: "ok" };
      else if (typeof r.httpStatus === "number") verdict = { state: "http", status: r.httpStatus };
      else verdict = { state: "network" };
    } catch {
      verdict = { state: "network" };
    }
    tests.update((m) => ({ ...m, [id]: verdict }));
  }

  /// The plain-language result line for a finished test.
  function testLabel(v: TestVerdict): string {
    switch (v.state) {
      case "testing":
        return $t("s.prov.test.testing");
      case "ok":
        return $t("s.prov.test.works");
      case "network":
        return $t("s.prov.test.network");
      case "http":
        if (v.status === 401) return $t("s.prov.test.needsKey");
        if (v.status === 403) return $t("s.prov.test.notAllowed");
        if (v.status === 429) return $t("s.prov.test.rateLimited");
        return $t("s.prov.test.failed", { status: v.status });
    }
  }
</script>

<Page
  title={$t("s.prov.title")}
  description={$t("s.prov.desc")}
>
  <SectionGrid>
    <Group class="span-full">
      {#each $providers as p (p.id)}
        {@render row(p)}
      {/each}
      {#if loaded && $providers.length === 0}
        <p class="empty">{$t("s.prov.empty")}</p>
      {/if}
    </Group>
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
  {@const verdict = $tests[p.id]}
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
    {#snippet control()}
      <span class="pctl">
        {#if verdict && verdict.state !== "testing"}
          <span class="verdict">
            <span class="dot {verdict.state === 'ok' ? 'ok' : 'err'}" aria-hidden="true"></span>{testLabel(verdict)}
          </span>
        {/if}
        <Button
          variant={p.configured ? "outline" : "secondary"}
          size="sm"
          class="pbtn"
          disabled={verdict?.state === "testing"}
          onclick={() => testProvider(p.id)}
        >
          {verdict?.state === "testing" ? $t("s.prov.test.testing") : $t("s.prov.test.action")}
        </Button>
        <Switch value={p.enabled} ariaLabel={$t("s.prov.enable", { name: p.name })} onchange={(v) => setEnabled(p.id, v)} />
      </span>
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
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  /* The add escape hatch sits just under the catalogue, reading as secondary. */
  .add-row {
    display: flex;
    padding-top: 0.5rem;
  }
  /* The test verdict sits with the Test button in the control cluster. */
  .verdict {
    display: inline-flex;
    align-items: center;
    gap: 0.375rem;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    white-space: nowrap;
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: var(--radius-chip, 4px);
    flex-shrink: 0;
  }
  .dot.ok {
    background: var(--color-success);
  }
  .dot.err {
    background: var(--color-error);
  }
  /* The attribute chips (how you connect, plus any unofficial warning) sit in
     the row's preview slot, between the name and the control cluster. */
  .chips {
    display: inline-flex;
    align-items: center;
    gap: 0.375rem;
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
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .pctl {
    display: inline-flex;
    align-items: center;
    gap: 0.75rem;
  }
  /* Test and Connect share a width so the toggle column stays aligned down the
     list; Connect (the setup action) reads as secondary, Test (a check on an
     already-connected provider) stays a quiet outline. */
  :global(.pbtn) {
    min-width: 5.5rem;
  }
</style>
