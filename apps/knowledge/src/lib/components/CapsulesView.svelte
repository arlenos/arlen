<script lang="ts">
  /// The Capsules place (KA-R6): the active shares with a one-gesture revoke,
  /// and the composed mint surface (Tim's pick over a wizard) - what, who and
  /// how long, and the MANDATORY link-type preview, all visible at once. A
  /// risky high-degree link type starts excluded; including it is an explicit
  /// act. Revoke is terminal recall of future reads, said plainly, never
  /// "un-send".
  import { onMount } from "svelte";
  import { AlertTriangle, Check } from "lucide-svelte";
  import { ConfirmDialog } from "@arlen/ui-kit/components/ui/confirm-dialog";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { NumberInput } from "@arlen/ui-kit/components/ui/number-input";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import {
    capsules,
    capsulesMocked,
    capsulesUnavailable,
    actionFailed,
    loadCapsules,
    revokeCapsule,
    mintCapsule,
    previewFor,
    SHAREABLES,
    type Capsule,
    type LinkPreview,
  } from "$lib/stores/capsules";
  import { t } from "$lib/i18n/messages";

  onMount(loadCapsules);

  // The mint state: one composed block, preview refetches per scope pick.
  let scope = $state<string | null>(null);
  let audience = $state("assistant");
  let expiryDays = $state(7);
  let reads = $state(50);
  let links = $state<LinkPreview[]>([]);

  const AUDIENCES = $derived([
    { value: "assistant", label: $t("k.ca.audAssistant") },
    { value: "app", label: $t("k.ca.audApp") },
  ]);
  const EXPIRIES = $derived([
    { value: "1", label: $t("k.ca.exp1") },
    { value: "7", label: $t("k.ca.exp7") },
    { value: "30", label: $t("k.ca.exp30") },
  ]);

  async function pickScope(name: string): Promise<void> {
    scope = name;
    links = await previewFor(name);
  }
  function toggleLink(rel: string): void {
    links = links.map((l) => (l.relation === rel ? { ...l, included: !l.included } : l));
  }

  const totalNodes = $derived(links.filter((l) => l.included).reduce((n, l) => n + l.nodes, 0));

  async function mint(): Promise<void> {
    if (!scope) return;
    const aud = AUDIENCES.find((a) => a.value === audience)?.label ?? audience;
    await mintCapsule(scope, aud, expiryDays, reads, links);
    scope = null;
    links = [];
  }

  let pendingRevoke = $state<Capsule | null>(null);
  async function confirmRevoke(): Promise<void> {
    if (!pendingRevoke) return;
    await revokeCapsule(pendingRevoke.id);
    pendingRevoke = null;
  }
</script>

<div class="ca">
  <div class="ca-head">
    {#if $capsulesMocked}
      <span class="ca-sample">{$t("k.sample")}</span>
    {:else if $capsulesUnavailable}
      <span class="ca-sample">{$t("k.capsules.unavailable")}</span>
    {/if}
    <!-- An action that did not reach the broker says so here, next to the list
         it did not change. Silence would leave the previous state on screen
         reading as the result. -->
    {#if $actionFailed}
      <span class="ca-sample" role="alert">
        {$t($actionFailed === "mint" ? "k.capsules.mintFailed" : "k.capsules.revokeFailed")}
      </span>
    {/if}
  </div>

  <div class="ca-scroll">
    <section class="ca-block">
      <h2 class="ca-block-head">{$t("k.ca.active")}</h2>
      {#if $capsules && $capsules.length === 0}
        <!-- The header badge above says the list could not be read; this line is
             where a reader actually looks, so it carries the distinction too. -->
        <p class="ca-empty">{$capsulesUnavailable ? $t("k.capsules.unavailable") : $t("k.empty.capsules")}</p>
      {:else if $capsules}
        <div class="ca-list">
          {#each $capsules as c (c.id)}
            <div class="ca-row">
              <span class="ca-label">{c.label}</span>
              <span class="ca-aud">{c.audience}</span>
              <span class="ca-meta">{$t("k.ca.rowMeta", { expires: c.expiresAt, reads: c.readsLeft })}</span>
              <button type="button" class="ca-revoke" onclick={() => (pendingRevoke = c)}>
                {$t("k.ca.revoke")}
              </button>
            </div>
          {/each}
        </div>
      {/if}
    </section>

    <section class="ca-block">
      <h2 class="ca-block-head">{$t("k.ca.mint")}</h2>

      <div class="ca-field">
        <span class="ca-field-label">{$t("k.ca.what")}</span>
        <div class="ca-scopes">
          {#each SHAREABLES as s (s.name)}
            <button type="button" class="ca-scope" class:picked={scope === s.name} onclick={() => pickScope(s.name)}>
              <span class="ca-scope-kind">{$t(s.kind === "project" ? "k.ca.kindProject" : "k.ca.kindSearch")}</span>
              {s.name}
            </button>
          {/each}
        </div>
      </div>

      <div class="ca-field">
        <span class="ca-field-label">{$t("k.ca.who")}</span>
        <div class="ca-grant">
          <PopoverSelect value={audience} options={AUDIENCES} ariaLabel={$t("k.ca.who")} width="12rem" onchange={(v) => (audience = v)} />
          <PopoverSelect value={String(expiryDays)} options={EXPIRIES} ariaLabel={$t("k.ca.expiry")} width="9rem" onchange={(v) => (expiryDays = Number(v))} />
          <NumberInput value={reads} min={1} max={500} unit={$t("k.ca.reads")} width="10rem" ariaLabel={$t("k.ca.reads")} onchange={(v) => (reads = v)} />
        </div>
      </div>

      {#if scope}
        <div class="ca-field">
          <span class="ca-field-label">{$t("k.ca.preview")}</span>
          <div class="ca-links">
            {#each links as l (l.relation)}
              <button type="button" class="ca-link" class:off={!l.included} onclick={() => toggleLink(l.relation)}>
                <span class="ca-link-state" class:risky={l.risky}>
                  {#if l.included}<Check size={13} strokeWidth={2} />{:else if l.risky}<AlertTriangle size={13} strokeWidth={2} />{/if}
                </span>
                <span class="ca-link-text">{$t("k.ca.follows", { relation: l.relation, n: l.nodes })}</span>
                <span class="ca-link-action">{l.included ? $t("k.ca.drop") : $t("k.ca.include")}</span>
              </button>
            {/each}
          </div>
          <p class="ca-total">{$t("k.ca.total", { n: totalNodes })}</p>
        </div>

        <div class="ca-mint-row">
          <Button onclick={mint} disabled={totalNodes === 0}>{$t("k.ca.mintBtn")}</Button>
        </div>
      {:else}
        <p class="ca-hint">{$t("k.ca.pickFirst")}</p>
      {/if}
    </section>
  </div>
</div>

<ConfirmDialog
  open={pendingRevoke !== null}
  title={$t("k.ca.revokeTitle")}
  message={$t("k.ca.revokeMsg", { label: pendingRevoke?.label ?? "", audience: pendingRevoke?.audience ?? "" })}
  confirmLabel={$t("k.ca.revoke")}
  variant="destructive"
  onConfirm={confirmRevoke}
  onCancel={() => (pendingRevoke = null)}
/>

<style>
  .ca {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .ca-head {
    display: flex;
    align-items: center;
    padding: 0.6rem 1.1rem 0;
  }
  .ca-sample {
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  .ca-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0.25rem 1.1rem 1.5rem;
    max-width: 46rem;
  }

  .ca-block {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    margin-top: 1rem;
  }
  .ca-block-head {
    margin: 0;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .ca-empty,
  .ca-hint {
    margin: 0;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }

  /* Active capsules: label, audience, the grant's remaining life, revoke. */
  .ca-list {
    display: flex;
    flex-direction: column;
  }
  .ca-row {
    display: grid;
    grid-template-columns: minmax(0, 1.1fr) minmax(0, 0.9fr) max-content 4.5rem;
    align-items: baseline;
    column-gap: 0.75rem;
    padding: 0.4rem 0.375rem;
    border-radius: var(--radius-chip, 4px);
  }
  .ca-row:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 4%, transparent);
  }
  .ca-label {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--color-fg-primary);
  }
  .ca-aud {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .ca-meta {
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
    white-space: nowrap;
  }
  .ca-revoke {
    justify-self: end;
    border: none;
    background: transparent;
    padding: 0.125rem 0.25rem;
    font-size: var(--text-xs);
    font-weight: 500;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
    cursor: pointer;
    transition: color var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .ca-revoke:hover {
    color: var(--color-error, #dc2626);
  }

  /* The mint blocks: label column then the control cluster. */
  .ca-field {
    display: flex;
    flex-direction: column;
    gap: 0.375rem;
    margin-top: 0.5rem;
  }
  .ca-field-label {
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-primary) 40%, transparent);
  }
  .ca-scopes {
    display: flex;
    flex-wrap: wrap;
    gap: 0.375rem;
  }
  .ca-scope {
    display: inline-flex;
    align-items: baseline;
    gap: 0.4rem;
    padding: 0.3rem 0.625rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 14%, transparent);
    border-radius: var(--radius-button, 6px);
    background: transparent;
    font-size: var(--text-xs);
    font-weight: 500;
    color: var(--color-fg-primary);
    cursor: pointer;
  }
  .ca-scope.picked {
    border-color: color-mix(in srgb, var(--color-accent, #6aa9e0) 45%, transparent);
    background: color-mix(in srgb, var(--color-accent, #6aa9e0) 10%, transparent);
  }
  .ca-scope-kind {
    font-size: var(--text-2xs);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: color-mix(in srgb, var(--color-fg-primary) 40%, transparent);
  }
  .ca-grant {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  /* The mandatory preview: one row per link type the walk would follow; a
     risky row is excluded until the user includes it deliberately. */
  .ca-links {
    display: flex;
    flex-direction: column;
  }
  .ca-link {
    display: grid;
    grid-template-columns: 1.25rem minmax(0, 1fr) max-content;
    align-items: center;
    column-gap: 0.5rem;
    padding: 0.3rem 0.375rem;
    border: none;
    border-radius: var(--radius-chip, 4px);
    background: transparent;
    text-align: start;
    cursor: pointer;
  }
  .ca-link:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 5%, transparent);
  }
  .ca-link.off .ca-link-text {
    color: color-mix(in srgb, var(--color-fg-primary) 40%, transparent);
  }
  .ca-link-state {
    display: inline-flex;
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .ca-link-state.risky {
    color: var(--color-warning, #ca8a04);
  }
  .ca-link-text {
    font-size: var(--text-sm);
    color: var(--color-fg-primary);
  }
  .ca-link-action {
    font-size: var(--text-xs);
    font-weight: 500;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .ca-total {
    margin: 0.125rem 0 0;
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  .ca-mint-row {
    display: flex;
    justify-content: flex-end;
    margin-top: 0.75rem;
  }
</style>
