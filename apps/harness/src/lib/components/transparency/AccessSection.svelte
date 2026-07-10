<script lang="ts">
  /// Access: what the AI can reach. The read tier and action mode in the
  /// same plain words the conversation surface uses (one datum, two
  /// contexts, kg-surface-allocation.md §3), then the AI's own grants
  /// grouped by principal with their reachable entity types. Read-only:
  /// revoking lives in Settings, never here. Rendering only; the page
  /// owns the reads.
  import { t } from "$lib/i18n/messages";
  import { statusSentence, statusTooltip } from "$lib/display";
  import type { Capability } from "$lib/capability";
  import { principalLabel, reachLabel, type GrantView } from "$lib/transparency";
  import SectionState from "./SectionState.svelte";

  let {
    grants,
    capability,
    loaded,
  }: {
    /// The AI-scoped grants, `null` while unloaded or after a failed read.
    grants: GrantView[] | null;
    capability: Capability | null;
    /// First read settled.
    loaded: boolean;
  } = $props();

  const off = $derived(capability !== null && !capability.enabled);

  // One entry per principal: its label and the union of reachable types
  // across that principal's live, non-revoked grants. A revoked or
  // superseded grant is not a current reach, so it drops out.
  const principals = $derived.by(() => {
    const active = (grants ?? []).filter((g) => !g.revoked && !g.superseded);
    const byApp = new Map<string, { label: string; reach: string[] }>();
    for (const g of active) {
      const entry = byApp.get(g.app_id) ?? { label: principalLabel(g.app_id), reach: [] };
      for (const r of g.reach) if (!entry.reach.includes(r)) entry.reach.push(r);
      byApp.set(g.app_id, entry);
    }
    return [...byApp.values()];
  });
</script>

<div class="access" id="transparency-access">
  {#if capability}
    <p class="tier" title={statusTooltip(capability, $t)}>{statusSentence(capability, $t)}</p>
  {/if}

  {#if !loaded}
    <SectionState message={$t("h.access.checking")} />
  {:else if grants === null}
    <SectionState message={$t("h.access.cantRead")} />
  {:else if principals.length === 0}
    <SectionState message={$t("h.access.none")} />
  {:else}
    {#if off}
      <p class="inactive">While the AI is off, none of this access is active.</p>
    {/if}
    <ul class="grants">
      {#each principals as p (p.label)}
        <li class="grant">
          <span class="who">{p.label}</span>
          <span class="reach">
            {#if p.reach.length === 0}
              <span class="none">no data</span>
            {:else}
              {#each p.reach as r (r)}
                <span class="chip">{reachLabel(r)}</span>
              {/each}
            {/if}
          </span>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .access {
    display: flex;
    flex-direction: column;
  }
  /* The tier line shares the section's voice with the chat capability
     strip: one plain sentence, the technical facts in its tooltip. */
  .tier {
    margin: 0;
    padding: 0.625rem var(--space-row, 0.75rem);
    font-size: 0.8125rem;
    line-height: 1.5;
    color: var(--foreground);
    border-bottom: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
  }
  .inactive {
    margin: 0;
    padding: 0.5rem var(--space-row, 0.75rem) 0;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .grants {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .grant {
    display: grid;
    grid-template-columns: 11rem minmax(0, 1fr);
    align-items: start;
    column-gap: var(--space-row, 0.75rem);
    padding: 0.625rem var(--space-row, 0.75rem);
  }
  .grant + .grant {
    border-top: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
  }
  .who {
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--foreground);
  }
  .reach {
    display: flex;
    flex-wrap: wrap;
    gap: 0.375rem;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    height: var(--height-tag, 20px);
    padding: 0 0.5rem;
    border-radius: var(--radius-chip);
    font-size: 0.6875rem;
    font-weight: 500;
    color: color-mix(in srgb, var(--foreground) 75%, transparent);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  .none {
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
</style>
