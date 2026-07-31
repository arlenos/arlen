<script lang="ts">
  /// Action items (meetings-app.md killer #3): attribution is a draft, never an
  /// assertion - the owner is an editable chip ("Set owner" when the model found
  /// none), the checkbox is real state, and a grounded item clicks through to its
  /// transcript span.
  import { Checkbox } from "@arlen/ui-kit/components/ui/checkbox";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { t } from "$lib/i18n/messages";
  import type { ActionItem, Transcript } from "$lib/contract";

  let {
    items,
    transcript,
    onjump,
    onupdate,
  }: {
    items: ActionItem[];
    transcript: Transcript;
    onjump: (startMs: number) => void;
    onupdate: (index: number, patch: { owner?: string; done?: boolean }) => void;
  } = $props();

  let editingOwner = $state<number | null>(null);
  let ownerDraft = $state("");

  function beginOwner(i: number, current: string | undefined) {
    editingOwner = i;
    ownerDraft = current ?? "";
  }
  function commitOwner(i: number) {
    editingOwner = null;
    onupdate(i, { owner: ownerDraft.trim() || undefined });
  }

  function startOf(item: ActionItem): number | null {
    if (item.source_segment === undefined) return null;
    const seg = transcript.segments[item.source_segment];
    return seg ? seg.start_ms : null;
  }
</script>

<section class="items" aria-label={$t("mt.actionItems")}>
  <span class="sec-label">{$t("mt.actionItems")}</span>
  {#if items.length === 0}
    <p class="none">{$t("mt.actionItems.none")}</p>
  {:else}
    {#each items as item, i (item.text)}
      {@const start = startOf(item)}
      <div class="item">
        <Checkbox
          id={`item-done-${i}`}
          checked={item.done ?? false}
          ariaLabel={item.text}
          onchange={(v) => onupdate(i, { done: v })}
        />
        {#if start !== null}
          <button type="button" class="item-text linked" class:done={item.done} onclick={() => onjump(start)}>
            {item.text}
          </button>
        {:else}
          <span class="item-text" class:done={item.done}>{item.text}</span>
        {/if}
        {#if editingOwner === i}
          <span class="owner-edit">
            <Input
              value={ownerDraft}
              aria-label={$t("mt.owner")}
              class="h-7 w-32 text-xs"
              oninput={(e) => (ownerDraft = e.currentTarget.value)}
              onkeydown={(e) => {
                if (e.key === "Enter") commitOwner(i);
                if (e.key === "Escape") editingOwner = null;
              }}
            />
          </span>
        {:else}
          <button
            type="button"
            class="owner"
            class:unset={!item.owner}
            id={`item-owner-${i}`}
            title={$t("mt.owner.edit")}
            onclick={() => beginOwner(i, item.owner)}
          >
            {item.owner ?? $t("mt.owner.set")}
          </button>
        {/if}
      </div>
    {/each}
  {/if}
</section>

<style>
  .items {
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
  }
  .sec-label {
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .none {
    margin: 0;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .item {
    display: flex;
    align-items: center;
    gap: 0.6rem;
  }
  .item-text {
    flex: 1;
    min-width: 0;
    padding: 0;
    border: none;
    background: transparent;
    text-align: start;
    font-size: var(--text-sm);
    line-height: 1.5;
    color: var(--color-fg-primary);
  }
  .item-text.linked {
    cursor: pointer;
  }
  .item-text.linked:hover {
    text-decoration: underline;
    text-underline-offset: 3px;
  }
  .item-text.done {
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
    text-decoration: line-through;
  }
  /* The owner draft: a quiet chip; unset shows the invitation, never a guess. */
  .owner {
    flex-shrink: 0;
    padding: 0.15rem 0.55rem;
    border: none;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-fg-primary) 75%, transparent);
    cursor: pointer;
  }
  .owner:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 13%, transparent);
  }
  .owner.unset {
    background: transparent;
    border: 1px dashed color-mix(in srgb, var(--color-fg-primary) 25%, transparent);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
</style>
