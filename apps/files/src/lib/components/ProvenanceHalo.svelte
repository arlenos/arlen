<script lang="ts">
  /// The provenance halo (provenance-halo.md, PH-R4): a pull-only, dismissable
  /// micro-surface summoned by a calm gesture, answering where a file came from in
  /// plain language. Model-free, offline, never steals focus. The honesty lives in
  /// the store's phrasing (unsigned trust-on-assertion, never "verified" unless a
  /// content credential backs it); this just renders the sentences calmly, no icons.
  import * as Popover from "@arlen/ui-kit/components/ui/popover";
  import { writable } from "svelte/store";
  import {
    loadProvenance,
    stepLine,
    horizonLine,
    type ProvenanceChain,
  } from "$lib/stores/provenance";
  import { t } from "$lib/i18n/messages";

  let { fileRef }: { fileRef: string } = $props();

  let open = $state(false);
  // IPC result through a writable, not $state (the Svelte-5 re-render caveat).
  const chain = writable<ProvenanceChain | null>(null);

  $effect(() => {
    if (open) {
      chain.set(null);
      void loadProvenance(fileRef).then((c) => chain.set(c));
    }
  });
</script>

<Popover.Root bind:open>
  <Popover.Trigger class="ph-trigger">{$t("f.prov.trigger")}</Popover.Trigger>
  <Popover.Content align="start" sideOffset={6} class="ph-pop">
    {#if $chain}
      <div class="ph-subject">{$chain.subject}</div>
      <div class="ph-steps">
        {#each $chain.steps as s, i (i)}
          <p class="ph-step" class:attested={s.attested}>{stepLine(s)}</p>
        {/each}
      </div>
      {#if horizonLine($chain)}
        <p class="ph-horizon">{horizonLine($chain)}</p>
      {/if}
    {:else}
      <p class="ph-loading">{$t("f.prov.loading")}</p>
    {/if}
  </Popover.Content>
</Popover.Root>

<style>
  :global(.ph-trigger) {
    padding: 0;
    border: none;
    background: transparent;
    font-size: var(--text-xs);
    font-weight: 500;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    cursor: pointer;
    text-align: start;
  }
  :global(.ph-trigger:hover) {
    color: var(--foreground);
  }

  :global(.ph-pop) {
    width: 17rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .ph-subject {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ph-steps {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  .ph-step {
    margin: 0;
    font-size: var(--text-sm);
    line-height: 1.45;
    color: var(--foreground);
  }
  /* The one origin with real backing reads a touch stronger; everything else stays
     trust-on-assertion prose. */
  .ph-step.attested {
    color: var(--foreground);
  }
  .ph-horizon {
    margin: 0;
    padding-top: 0.15rem;
    font-size: var(--text-2xs);
    line-height: 1.4;
    color: color-mix(in srgb, var(--foreground) 42%, transparent);
    border-top: 1px solid color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  .ph-loading {
    margin: 0;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
</style>
