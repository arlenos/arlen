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
  import { ProviderLogo } from "@arlen/ui-kit/components/ui/provider-logo";

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

  async function loadProviders() {
    try {
      providers.set(parse<Provider[]>(await invoke<string>("ai_providers_list"), []));
    } catch {
      providers.set([]);
    } finally {
      loaded = true;
    }
  }
  onMount(loadProviders);

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
    if (p.kind === "local") return "Local, no egress";
    return p.region ? `Cloud, ${p.region}` : "Cloud, egress audited";
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
        return "Testing…";
      case "ok":
        return "Connection works";
      case "network":
        return "Could not reach it";
      case "http":
        if (v.status === 401) return "Needs a key";
        if (v.status === 403) return "Not allowed (403)";
        if (v.status === 429) return "Rate limited (429)";
        return `Failed (${v.status})`;
    }
  }
</script>

<Page
  title="Providers"
  description="Connect the AI services the assistant may use. Keys are held in the system keystore, never by the assistant. Choosing which model answers lives on the Default models page."
>
  <SectionGrid>
    <Group class="span-full">
      {#each $providers as p (p.id)}
        {@render row(p)}
      {/each}
      {#if loaded && $providers.length === 0}
        <p class="empty">No providers are set up yet.</p>
      {/if}
    </Group>
  </SectionGrid>
</Page>

{#snippet row(p: Provider)}
  {@const verdict = $tests[p.id]}
  <Row label={p.name} description={meta(p)} id={`provider-${p.id}`}>
    {#snippet leading()}
      <ProviderLogo id={p.id} name={p.name} size={24} />
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
          {verdict?.state === "testing" ? "Testing…" : "Test"}
        </Button>
        <Switch value={p.enabled} ariaLabel={`Enable ${p.name}`} onchange={(v) => setEnabled(p.id, v)} />
      </span>
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
