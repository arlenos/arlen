<script lang="ts">
  /// The capsule mint flow: a three-step dialog to share a slice of your context.
  /// Step 1 picks a named thing, step 2 sets the recipient + a mandatory expiry +
  /// an op-count, step 3 is the mandatory over-share preview (drop any relation you
  /// do not want, personal fields off by default) and the Share action. Mounted
  /// once in the layout, opened from the sidebar. Fixture-backed; mint is a human
  /// act, never an agent path.
  import Dialog from "@arlen/ui-kit/components/ui/dialog/dialog.svelte";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Checkbox } from "@arlen/ui-kit/components/ui/checkbox";
  import ChoiceList from "@arlen/ui-kit/components/ui/choice-list/choice-list.svelte";
  import PopoverSelect from "@arlen/ui-kit/components/ui/popover-select/popover-select.svelte";
  import { ShieldCheck } from "lucide-svelte";
  import {
    mintOpen,
    mintStep,
    mintForm,
    scopeOptions,
    preview,
    mintResult,
    closeMint,
    loadPreview,
    mint,
    type MintForm,
  } from "$lib/stores/mint";

  const STEPS = ["Choose what to share", "Recipient and limits", "Review and share"];
  const EXPIRY = [
    { value: "1d", label: "1 day" },
    { value: "1w", label: "1 week" },
    { value: "1m", label: "1 month" },
  ];
  const OPCOUNT = [
    { value: "5", label: "5 reads" },
    { value: "20", label: "20 reads" },
    { value: "100", label: "100 reads" },
  ];
  const AUDIENCE = [
    { value: "this-machine", label: "This machine", description: "Only this computer can open the share." },
    { value: "paired", label: "A paired device", description: "A device you have linked. Available once device pairing ships." },
    { value: "key", label: "A recipient's key", description: "Bind the share to someone's key. Available once external sharing ships." },
  ];

  const scopeLabel = $derived($scopeOptions.find((o) => o.id === $mintForm.scopeId)?.label ?? "");
  const audienceLabel = $derived(AUDIENCE.find((a) => a.value === $mintForm.audience)?.label ?? "");
  const includedRelations = $derived(
    ($preview?.relations ?? []).filter((r) => !$mintForm.dropped.includes(r.type)),
  );
  const totalItems = $derived(
    ($preview?.baseCount ?? 0) + includedRelations.reduce((s, r) => s + r.reach, 0),
  );
  const canNext = $derived(($mintStep === 0 && !!$mintForm.scopeId) || $mintStep === 1);

  function setForm(patch: Partial<MintForm>) {
    mintForm.update((f) => ({ ...f, ...patch }));
  }
  function toggleRelation(type: string) {
    mintForm.update((f) => ({
      ...f,
      dropped: f.dropped.includes(type) ? f.dropped.filter((t) => t !== type) : [...f.dropped, type],
    }));
  }
  function next() {
    if ($mintStep === 1 && $mintForm.scopeId) void loadPreview($mintForm.scopeId);
    mintStep.set(Math.min(2, $mintStep + 1));
  }
  function back() {
    mintStep.set(Math.max(0, $mintStep - 1));
  }
  async function share() {
    await mint($mintForm, scopeLabel);
  }
</script>

<Dialog open={$mintOpen} onClose={closeMint} size="lg" ariaLabel="Share context">
  <div class="mint">
    <header class="mint-head">
      {#if $mintResult}
        <h2 class="mint-title">Share created</h2>
      {:else}
        <span class="mint-eyebrow">Step {$mintStep + 1} of 3</span>
        <h2 class="mint-title">{STEPS[$mintStep]}</h2>
      {/if}
    </header>

    <div class="mint-body">
      {#if $mintResult}
        <div class="mint-result">
          <span class="mint-result-icon"><ShieldCheck size={20} strokeWidth={1.75} /></span>
          <p class="mint-lead"><strong>{$mintResult}</strong> is now shared with {audienceLabel.toLowerCase()}.</p>
          <p class="mint-hint">
            You can see it and revoke it any time under Settings, Privacy, Shared
            context. Revoking stops any further reads.
          </p>
        </div>
      {:else if $mintStep === 0}
        <p class="mint-lead">
          Pick a named thing to share. You share a snapshot of it as it is now,
          never a live feed.
        </p>
        <ChoiceList
          value={$mintForm.scopeId ?? ""}
          options={$scopeOptions.map((o) => ({ value: o.id, label: o.label, description: o.description }))}
          ariaLabel="What to share"
          onchange={(v) => setForm({ scopeId: v })}
        />
      {:else if $mintStep === 1}
        <div class="mint-field">
          <span class="mint-label">Who can open it</span>
          <ChoiceList
            value={$mintForm.audience}
            options={AUDIENCE}
            ariaLabel="Recipient"
            onchange={(v) => setForm({ audience: v })}
          />
        </div>
        <div class="mint-row2">
          <div class="mint-field">
            <span class="mint-label">Expires</span>
            <PopoverSelect value={$mintForm.expiry} options={EXPIRY} ariaLabel="Expiry" onchange={(v) => setForm({ expiry: v })} />
            <span class="mint-hint">A share always expires. There is no permanent share.</span>
          </div>
          <div class="mint-field">
            <span class="mint-label">Good for</span>
            <PopoverSelect value={$mintForm.opCount} options={OPCOUNT} ariaLabel="Reads" onchange={(v) => setForm({ opCount: v })} />
            <span class="mint-hint">How many times the recipient can open it.</span>
          </div>
        </div>
      {:else if $preview === null}
        <p class="mint-lead">Checking what this share would include…</p>
      {:else}
        <p class="mint-lead">
          Sharing <strong>{scopeLabel}</strong> follows these connections. Drop any
          you do not want to include.
        </p>
        <div class="rel-list">
          <div class="rel">
            <Checkbox checked disabled ariaLabel={scopeLabel} />
            <span class="rel-what">{scopeLabel}</span>
            <span class="rel-reach">{$preview.baseCount} items</span>
          </div>
          {#each $preview.relations as r (r.type)}
            {@const on = !$mintForm.dropped.includes(r.type)}
            <label class="rel" class:off={!on}>
              <Checkbox checked={on} onchange={() => toggleRelation(r.type)} ariaLabel={`Include ${r.label}`} />
              <span class="rel-what">{r.label}</span>
              <span class="rel-reach">{r.reach.toLocaleString()} {r.reach === 1 ? "item" : "items"}</span>
            </label>
          {/each}
        </div>
        <label class="sensitive">
          <Checkbox checked={$mintForm.includeSensitive} onchange={(v) => setForm({ includeSensitive: v })} ariaLabel="Include personal fields" />
          <span>Include personal fields, like email and phone. Off by default.</span>
        </label>
        <p class="mint-summary">
          This share includes <strong>{totalItems.toLocaleString()} items</strong>
          across {includedRelations.length}
          {includedRelations.length === 1 ? "connection" : "connections"}, readable
          by {audienceLabel.toLowerCase()} until it expires.
        </p>
      {/if}
    </div>

    <footer class="mint-foot">
      {#if $mintResult}
        <span class="mint-spacer"></span>
        <Button onclick={closeMint}>Done</Button>
      {:else}
        {#if $mintStep > 0}
          <Button variant="ghost" onclick={back}>Back</Button>
        {/if}
        <span class="mint-spacer"></span>
        <Button variant="ghost" onclick={closeMint}>Cancel</Button>
        {#if $mintStep < 2}
          <Button onclick={next} disabled={!canNext}>Next</Button>
        {:else}
          <Button onclick={share} disabled={$preview === null}>Share</Button>
        {/if}
      {/if}
    </footer>
  </div>
</Dialog>

<style>
  .mint {
    display: flex;
    flex-direction: column;
    max-height: min(80vh, 640px);
  }
  .mint-head {
    padding: 1.25rem 1.25rem 0.75rem;
  }
  .mint-eyebrow {
    display: block;
    font-size: 0.6875rem;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .mint-title {
    margin: 0.15rem 0 0;
    font-size: 1.0625rem;
    font-weight: 600;
    color: var(--foreground);
  }
  .mint-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0.875rem;
    padding: 0.25rem 1.25rem 0.5rem;
  }
  .mint-lead {
    margin: 0;
    font-size: 0.8125rem;
    line-height: 1.5;
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
  }
  .mint-lead strong {
    color: var(--foreground);
    font-weight: 600;
  }
  .mint-field {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  .mint-label {
    font-size: 0.75rem;
    font-weight: 600;
    color: var(--foreground);
  }
  .mint-row2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 1rem;
  }
  .mint-hint {
    font-size: 0.6875rem;
    line-height: 1.4;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }

  /* The over-share preview: a bordered container that sets the concentric radius
     for its rows (the ChoiceList convention), each row a drop toggle. */
  .rel-list {
    display: flex;
    flex-direction: column;
    padding: 4px;
    border-radius: var(--radius-input);
    border: 1px solid color-mix(in srgb, var(--foreground) 10%, transparent);
    background: color-mix(in srgb, var(--foreground) 3%, transparent);
    --container-radius: var(--radius-input);
    --container-inset: 4px;
  }
  .rel {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr) max-content;
    align-items: center;
    gap: 0.625rem;
    width: 100%;
    padding: 0.5rem 0.625rem;
    border: none;
    background: transparent;
    border-radius: max(0px, calc(var(--container-radius) - var(--container-inset)));
    text-align: left;
    transition: background-color var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  label.rel {
    cursor: pointer;
  }
  label.rel:hover {
    background: color-mix(in srgb, var(--foreground) 6%, transparent);
  }
  .rel.off .rel-what,
  .rel.off .rel-reach {
    opacity: 0.4;
    text-decoration: line-through;
  }
  .rel-what {
    font-size: 0.8125rem;
    color: var(--foreground);
    min-width: 0;
  }
  .rel-reach {
    justify-self: end;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .sensitive {
    display: flex;
    align-items: center;
    gap: 0.625rem;
    padding: 0.25rem 0.125rem;
    cursor: pointer;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
    cursor: pointer;
  }

  .mint-summary {
    margin: 0;
    padding-top: 0.25rem;
    font-size: 0.8125rem;
    line-height: 1.5;
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
  }
  .mint-summary strong {
    color: var(--foreground);
    font-weight: 600;
  }

  .mint-result {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: 0.5rem;
    padding: 1rem 0.5rem;
  }
  .mint-result-icon {
    display: inline-flex;
    color: var(--color-success);
  }

  .mint-foot {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.75rem 1.25rem 1.25rem;
    border-top: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
  }
  .mint-spacer {
    flex: 1;
  }
</style>
