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
  import { Plus } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { Button } from "@arlen/ui-kit/components/ui/button";
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

  let addOpen = $state(false);
</script>

<Page
  title="AI providers"
  description="Connect the AI services the assistant may use. Keys are held in the system keystore, never by the assistant. Choosing which model answers lives on the Default models page."
>
  <SectionGrid>
    <Group class="span-full">
      {#each $providers as p (p.id)}
        {@render row(p)}
      {/each}
      {#if loaded && $providers.length === 0}
        <p class="empty">No providers are set up yet. Add one to get started.</p>
      {/if}
      <button type="button" class="add-row" onclick={() => (addOpen = true)}>
        <span class="add-logo" aria-hidden="true"><Plus size={14} strokeWidth={2} /></span>
        <span class="add-label">Add provider</span>
      </button>
    </Group>
  </SectionGrid>
</Page>

<AddProviderDialog open={addOpen} onClose={() => (addOpen = false)} />

{#snippet row(p: Provider)}
  <div class="prow">
    <span class="plogo" aria-hidden="true">{p.name.charAt(0).toUpperCase()}</span>
    <div class="pmeta">
      <div class="pname">{p.name}</div>
      <div class="pdesc">
        {meta(p)}
        {#if p.configured && p.status === "ok"}
          <span class="dot ok" aria-label="Connected"></span>Connected
        {:else if p.status === "error"}
          <span class="dot err" aria-label="Connection failed"></span>Connection failed
        {/if}
      </div>
    </div>
    <div class="pctl">
      {#if p.configured}
        <Button variant="outline" size="sm" class="pbtn">Test</Button>
      {:else}
        <Button variant="secondary" size="sm" class="pbtn">Connect</Button>
      {/if}
      <Switch value={p.enabled} ariaLabel={`Enable ${p.name}`} onchange={(v) => setEnabled(p.id, v)} />
    </div>
  </div>
{/snippet}

<style>
  .empty {
    margin: 0;
    padding: var(--space-row, 0.75rem) 1rem;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  /* A provider row matches the kit Row metrics (Group divides direct children
     for us), with a leading logo the standard Row has no slot for. */
  .prow {
    display: flex;
    align-items: center;
    gap: 0.875rem;
    padding: var(--space-row, 0.75rem) 1rem;
    min-height: var(--height-row, 40px);
  }
  /* The provider logo slot: mocked as the initial in a rounded tile, the same
     placeholder the in-chat picker uses; the real @lobehub/icons set drops in
     here. The tile stays the fallback for a provider with no brand asset. */
  .plogo {
    flex-shrink: 0;
    width: 1.5rem;
    height: 1.5rem;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--foreground) 12%, transparent);
    font-size: 0.75rem;
    font-weight: 600;
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
  }
  .pmeta {
    flex: 1;
    min-width: 0;
  }
  .pname {
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--foreground);
    line-height: 1.3;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pdesc {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    font-size: 0.6875rem;
    line-height: 1.3;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    margin-top: 0.0625rem;
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: var(--radius-full, 9999px);
    flex-shrink: 0;
  }
  .dot.ok {
    background: var(--color-success);
  }
  .dot.err {
    background: var(--color-error);
  }
  .pctl {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    flex-shrink: 0;
  }
  /* Test and Connect share a width so the toggle column stays aligned down the
     list; Connect (the setup action) reads as secondary, Test (a check on an
     already-connected provider) stays a quiet outline. */
  :global(.pbtn) {
    min-width: 5.5rem;
  }
  /* The add-provider escape hatch reads as a quiet row, not a loud button. */
  .add-row {
    display: flex;
    align-items: center;
    gap: 0.875rem;
    width: 100%;
    padding: var(--space-row, 0.75rem) 1rem;
    min-height: var(--height-row, 40px);
    border: none;
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
    transition: color var(--duration-fast) var(--ease-out);
  }
  .add-row:hover {
    color: var(--foreground);
  }
  .add-logo {
    flex-shrink: 0;
    width: 1.5rem;
    height: 1.5rem;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-chip);
    border: 1px dashed color-mix(in srgb, var(--foreground) 25%, transparent);
  }
  .add-label {
    font-size: 0.8125rem;
    font-weight: 500;
  }
</style>
