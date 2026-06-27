<script lang="ts" module>
  /// One model the daemon can answer with, qualified by its provider. The
  /// same model name can appear under more than one provider (a local Ollama
  /// llama3 and a cloud one), so the active choice is always keyed on the
  /// pair, never the model alone. `local` carries the sovereignty signal
  /// (no egress vs audited egress); `available` is false for a listed model
  /// that cannot be used yet (e.g. a provider with no key).
  export interface ModelEntry {
    provider: string;
    model: string;
    contextWindow: number;
    /// Local provider (no egress) vs cloud (egress, audited) - the
    /// sovereignty signal. Mirrors the `ai_models_list` seam's `kind`.
    kind: "local" | "cloud";
    available: boolean;
  }
</script>

<script lang="ts">
  /// The in-chat model + provider picker (harness-redesign-plan.md, "Model +
  /// provider picker"). A quiet trigger at the composer's top edge names the
  /// active provider and model; clicking opens a searchable list, grouped by
  /// provider when more than one is present and flat when only one is. Each
  /// group carries its egress posture once. Picking swaps the model live for
  /// the turns that follow. Built on the kit Command + Popover canon.
  import { Check, ChevronDown, Cloud, House } from "@lucide/svelte";
  import * as Popover from "@arlen/ui-kit/components/ui/popover";
  import { ProviderLogo } from "@arlen/ui-kit/components/ui/provider-logo";
  import {
    Command,
    CommandInput,
    CommandList,
    CommandEmpty,
    CommandGroup,
    CommandItem,
  } from "@arlen/ui-kit/components/ui/command";

  let {
    models,
    active,
    onselect,
    disabled = false,
  }: {
    /// The catalogue from `ai_models_list`.
    models: ModelEntry[];
    /// The current `(provider, model)` from `ai_active`; null while loading.
    active: { provider: string; model: string } | null;
    /// Commit a live swap via `ai_set_active`.
    onselect: (provider: string, model: string) => void;
    disabled?: boolean;
  } = $props();

  let open = $state(false);

  // The providers in catalogue order, each with its models. A single provider
  // renders flat (no redundant heading); several render grouped.
  const providers = $derived.by(() => {
    const order: string[] = [];
    const byProvider = new Map<string, ModelEntry[]>();
    for (const m of models) {
      if (!byProvider.has(m.provider)) {
        byProvider.set(m.provider, []);
        order.push(m.provider);
      }
      byProvider.get(m.provider)!.push(m);
    }
    return order.map((p) => ({ provider: p, models: byProvider.get(p)! }));
  });
  const multi = $derived(providers.length > 1);

  const activeEntry = $derived(
    active ? models.find((m) => m.provider === active.provider && m.model === active.model) : null,
  );

  function choose(m: ModelEntry) {
    if (!m.available) return;
    onselect(m.provider, m.model);
    open = false;
  }

  function isActive(m: ModelEntry): boolean {
    return active?.provider === m.provider && active?.model === m.model;
  }

  // Context window in a compact form: 8000 -> "8k ctx", 128000 -> "128k ctx".
  function ctxLabel(n: number): string {
    return n >= 1000 ? `${Math.floor(n / 1000)}k ctx` : `${n} ctx`;
  }
</script>

<!-- The provider logo slot: the real brand mark from the kit, with the
     initial-tile fallback built into ProviderLogo. -->
{#snippet logo(provider: string)}
  <ProviderLogo id={provider} name={provider} size={16} />
{/snippet}

<Popover.Root bind:open>
  <Popover.Trigger disabled={disabled || models.length === 0}>
    {#snippet child({ props })}
      <button type="button" class="mp-trigger" {...props}>
        {#if activeEntry}
          {@render logo(activeEntry.provider)}
          <span class="mp-active">{activeEntry.provider} · {activeEntry.model}</span>
          {#if activeEntry.kind === "local"}
            <House size={11} strokeWidth={2} class="mp-kind" />
          {:else}
            <Cloud size={11} strokeWidth={2} class="mp-kind" />
          {/if}
        {:else}
          <span class="mp-active mp-empty">Choose a model</span>
        {/if}
        <ChevronDown size={12} strokeWidth={2} class="mp-chev" />
      </button>
    {/snippet}
  </Popover.Trigger>
  <Popover.Content side="top" align="start" sideOffset={6} class="mp-pop">
    <Command>
      <CommandInput placeholder="Search models" />
      <CommandList>
        <CommandEmpty>No models match.</CommandEmpty>
        {#each providers as group (group.provider)}
          <CommandGroup>
            {#if multi}
              {@const isLocal = group.models[0]?.kind === "local"}
              <div class="mp-group">
                {group.provider} · {isLocal ? "local · no egress" : "cloud · egress, audited"}
              </div>
            {/if}
            {#each group.models as m (m.provider + "/" + m.model)}
              <CommandItem
                value={`${m.provider} ${m.model}`}
                disabled={!m.available}
                onSelect={() => choose(m)}
              >
                {@render logo(m.provider)}
                <span class="mp-name">{m.model}</span>
                {#if !m.available}
                  <span class="mp-note">unavailable</span>
                {:else}
                  <span class="mp-ctx">{ctxLabel(m.contextWindow)}</span>
                {/if}
                <span class="mp-checkslot">
                  {#if isActive(m)}
                    <Check size={13} strokeWidth={2.5} class="mp-check" />
                  {/if}
                </span>
              </CommandItem>
            {/each}
          </CommandGroup>
        {/each}
      </CommandList>
    </Command>
  </Popover.Content>
</Popover.Root>

<style>
  .mp-trigger {
    display: inline-flex;
    align-items: center;
    gap: 0.375rem;
    max-width: 100%;
    height: var(--height-control, 28px);
    padding: 0 0.5rem;
    border: none;
    border-radius: var(--radius-button);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    font-size: 0.75rem;
    transition: color var(--duration-fast) var(--ease-out);
  }
  .mp-trigger:hover {
    color: color-mix(in srgb, var(--foreground) 85%, transparent);
  }
  .mp-active {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mp-empty {
    font-style: normal;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  :global(.mp-kind) {
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  :global(.mp-chev) {
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
  }
  /* The picker popover: a flat, opaque card (the kit popover-content leaves
     the background to the consumer); the kit Command paints the rest. */
  :global(.mp-pop) {
    width: 20rem;
    max-width: min(20rem, calc(100vw - 2rem));
    padding: 0;
    background: var(--color-bg-card);
    overflow: hidden;
  }
  /* The provider group header carries the egress posture once. */
  .mp-group {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    padding: 0.375rem 0.625rem 0.25rem;
    font-size: 0.6875rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .mp-group :global(svg) {
    flex-shrink: 0;
  }
  .mp-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mp-ctx {
    flex-shrink: 0;
    font-size: 0.6875rem;
    font-family: var(--font-mono, monospace);
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .mp-note {
    flex-shrink: 0;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
  }
  /* A fixed check gutter on every row so the ctx column stays aligned whether
     or not a row carries the active check. */
  .mp-checkslot {
    flex-shrink: 0;
    width: 0.875rem;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  :global(.mp-check) {
    flex-shrink: 0;
    color: var(--foreground);
  }
</style>
